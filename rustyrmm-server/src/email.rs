//! Email Notifications: the digest text, and the sender that drains the `email_outbox` table.

use std::time::Duration;

use chrono::{DateTime, Utc};
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sqlx::PgPool;
use uuid::Uuid;

const POLL: Duration = Duration::from_secs(15);
const BATCH: i64 = 20;
/// Total tries per email: now, then 1, 5 and 25 minutes after each failure.
const MAX_ATTEMPTS: i32 = 4;
/// How long a claimed row stays hidden; longer than `BATCH` sends at the 30 s SMTP timeout.
const LEASE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Tls {
    None,
    Starttls,
    Tls,
}

/// The `serve` flags for outgoing email. The password is only read from RUSTYRMM_SMTP_PASSWORD.
#[derive(clap::Args)]
pub struct SmtpArgs {
    /// SMTP server for emailing Notifications; email is off unless this and --smtp-from are set.
    #[arg(long, env = "RUSTYRMM_SMTP_HOST")]
    smtp_host: Option<String>,
    /// SMTP port; defaults to 25 for none, 587 for starttls, 465 for tls.
    #[arg(long, env = "RUSTYRMM_SMTP_PORT")]
    smtp_port: Option<u16>,
    /// How to secure the SMTP connection.
    #[arg(
        long,
        env = "RUSTYRMM_SMTP_TLS",
        value_enum,
        default_value = "starttls"
    )]
    smtp_tls: Tls,
    /// SMTP username (the password is read from RUSTYRMM_SMTP_PASSWORD).
    #[arg(long, env = "RUSTYRMM_SMTP_USERNAME")]
    smtp_username: Option<String>,
    /// Sender mailbox, e.g. "RustyRMM <rmm@example.com>".
    #[arg(long, env = "RUSTYRMM_SMTP_FROM")]
    smtp_from: Option<Mailbox>,
}

/// SMTP settings that are complete and ready to use.
pub struct Smtp {
    host: String,
    port: u16,
    tls: Tls,
    credentials: Option<Credentials>,
    from: Mailbox,
}

impl SmtpArgs {
    /// None when email is not configured. Errors when it is but --public-url is missing.
    pub fn resolve(self, public_url: Option<&str>) -> anyhow::Result<Option<Smtp>> {
        let (Some(host), Some(from)) = (self.smtp_host, self.smtp_from) else {
            return Ok(None);
        };
        if public_url.is_none() {
            anyhow::bail!("--public-url is required when --smtp-host and --smtp-from are set");
        }
        let port = self.smtp_port.unwrap_or(match self.smtp_tls {
            Tls::None => 25,
            Tls::Starttls => 587,
            Tls::Tls => 465,
        });
        let credentials = self.smtp_username.map(|user| {
            Credentials::new(
                user,
                std::env::var("RUSTYRMM_SMTP_PASSWORD").unwrap_or_default(),
            )
        });
        Ok(Some(Smtp {
            host,
            port,
            tls: self.smtp_tls,
            credentials,
            from,
        }))
    }
}

/// One Endpoint's change in a digest.
pub struct Change {
    pub endpoint_id: Uuid,
    pub hostname: String,
    pub last_seen_at: DateTime<Utc>,
    /// Some for "back online".
    pub offline_seconds: Option<i64>,
}

/// Replaces control characters (CR and LF included) so a name cannot add lines or headers.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn endpoints(n: usize) -> String {
    format!("{n} Endpoint{}", if n == 1 { "" } else { "s" })
}

/// The (subject, plain-text body) of one Operator's digest for one Environment.
pub fn digest(environment: &str, public_url: &str, changes: &[Change]) -> (String, String) {
    let environment = clean(environment);
    let (back, offline): (Vec<_>, Vec<_>) =
        changes.iter().partition(|c| c.offline_seconds.is_some());
    let subject = match (offline.len(), back.len()) {
        (n, 0) => format!("{} in {environment} went offline", endpoints(n)),
        (0, n) => format!("{} in {environment} back online", endpoints(n)),
        (n, m) => format!(
            "{environment}: {} went offline, {m} back online",
            endpoints(n)
        ),
    };
    let base = public_url.trim_end_matches('/');
    let mut body = String::new();
    for (title, list) in [("Went offline", offline), ("Back online", back)] {
        if list.is_empty() {
            continue;
        }
        body.push_str(&format!("{title}:\n"));
        for c in list {
            let seen = c.last_seen_at.format("%Y-%m-%d %H:%M UTC");
            let detail = match c.offline_seconds {
                Some(s) => format!("was offline {} min since {seen}", s / 60),
                None => format!("last seen {seen}"),
            };
            body.push_str(&format!(
                "- {} ({detail}) {base}/#/endpoints/{}\n",
                clean(&c.hostname),
                c.endpoint_id
            ));
        }
        body.push('\n');
    }
    (clean(&subject), body)
}

/// After a failed try (`attempts` counts it), how long to wait before the next; None = give up.
fn retry_delay(attempts: i32) -> Option<Duration> {
    (attempts < MAX_ATTEMPTS).then(|| Duration::from_secs(60 * 5u64.pow(attempts as u32 - 1)))
}

fn transport(
    smtp: &Smtp,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, lettre::transport::smtp::Error> {
    let builder = match smtp.tls {
        Tls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host),
        Tls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)?,
        Tls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)?,
    };
    let builder = builder
        .port(smtp.port)
        .timeout(Some(Duration::from_secs(30)));
    Ok(match &smtp.credentials {
        Some(c) => builder.credentials(c.clone()),
        None => builder,
    }
    .build())
}

/// Runs forever: sends the queued digests, retrying failures with backoff.
pub async fn run(pool: PgPool, smtp: Smtp) {
    let mailer = match transport(&smtp) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("email sender not started: {e}");
            return;
        }
    };
    let mut tick = tokio::time::interval(POLL);
    loop {
        tick.tick().await;
        if let Err(e) = send_due(&pool, &mailer, &smtp.from).await {
            // shortcut: stderr until the Server adopts structured logging.
            eprintln!("email job failed: {e}");
        }
    }
}

pub async fn send_due<T: AsyncTransport + Sync>(
    pool: &PgPool,
    mailer: &T,
    from: &Mailbox,
) -> sqlx::Result<()>
where
    T::Error: std::fmt::Display,
{
    let mut tx = pool.begin().await?;
    let due: Vec<(Uuid, Uuid, Option<String>, String, String, i32)> = sqlx::query_as(
        "SELECT b.id, b.operator_id, o.email, b.subject, b.body, b.attempts
         FROM email_outbox b JOIN operators o ON o.id = b.operator_id
         WHERE b.next_attempt_at <= now()
         ORDER BY b.next_attempt_at LIMIT $1
         FOR UPDATE OF b SKIP LOCKED",
    )
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;
    let ids: Vec<Uuid> = due.iter().map(|row| row.0).collect();
    sqlx::query(
        "UPDATE email_outbox SET next_attempt_at = now() + make_interval(secs => $2)
         WHERE id = ANY($1)",
    )
    .bind(ids)
    .bind(LEASE.as_secs_f64())
    .execute(&mut *tx)
    .await?;
    // Sends run with no transaction open, and each outcome is recorded right away, so a
    // later failure can't undo the record of an email SMTP already accepted.
    tx.commit().await?;
    for (id, operator_id, address, subject, body, attempts) in due {
        let Some(address) = address else {
            sqlx::query("DELETE FROM email_outbox WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await?;
            continue;
        };
        let sent = async {
            let to: Mailbox = address.parse().map_err(|e| format!("{e}"))?;
            let message = Message::builder()
                .from(from.clone())
                .to(to)
                .subject(clean(&subject))
                .header(ContentType::TEXT_PLAIN)
                .body(body)
                .map_err(|e| format!("{e}"))?;
            mailer.send(message).await.map_err(|e| format!("{e}"))
        }
        .await;
        match sent {
            Ok(_) => {
                sqlx::query("DELETE FROM email_outbox WHERE id = $1")
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            Err(error) => match retry_delay(attempts + 1) {
                Some(delay) => {
                    sqlx::query(
                        "UPDATE email_outbox SET attempts = attempts + 1,
                             next_attempt_at = now() + make_interval(secs => $2)
                         WHERE id = $1",
                    )
                    .bind(id)
                    .bind(delay.as_secs_f64())
                    .execute(pool)
                    .await?;
                }
                None => {
                    eprintln!(
                        "email to Operator {operator_id} dropped after {MAX_ATTEMPTS} attempts: {subject:?}: {error}"
                    );
                    sqlx::query("DELETE FROM email_outbox WHERE id = $1")
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
            },
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use lettre::transport::stub::AsyncStubTransport;

    fn change(hostname: &str, offline_seconds: Option<i64>) -> Change {
        Change {
            endpoint_id: Uuid::nil(),
            hostname: hostname.into(),
            last_seen_at: Utc::now(),
            offline_seconds,
        }
    }

    #[test]
    fn retries_back_off_one_five_then_twenty_five_minutes_and_stop() {
        let minutes = |n| retry_delay(n).map(|d| d.as_secs() / 60);
        assert_eq!(
            (minutes(1), minutes(2), minutes(3), minutes(4)),
            (Some(1), Some(5), Some(25), None)
        );
    }

    #[test]
    fn digest_text_has_no_line_breaks_from_names() {
        let changes = [
            change("web\r\nBcc: x@evil.test", None),
            change("db", Some(600)),
        ];
        let (subject, body) = digest(
            "Acme\r\nBcc: y@evil.test",
            "https://rmm.example.com/",
            &changes,
        );
        assert!(!subject.contains(['\r', '\n']));
        assert_eq!(
            subject,
            "Acme  Bcc: y@evil.test: 1 Endpoint went offline, 1 back online"
        );
        assert!(!body.contains('\r'));
        assert!(body.lines().all(|l| !l.starts_with("Bcc")));
        assert!(body.contains("https://rmm.example.com/#/endpoints/"));
        assert!(!body.contains("com//#"));
    }

    async fn queue(pool: &PgPool) -> Uuid {
        let op =
            crate::operators::add_operator(pool, "alice", "correct horse battery", "a@example.com")
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO email_outbox (id, operator_id, subject, body) VALUES ($1, $2, 's', 'b')",
        )
        .bind(Uuid::now_v7())
        .bind(op)
        .execute(pool)
        .await
        .unwrap();
        op
    }

    #[sqlx::test]
    async fn a_failed_send_is_retried_later_and_a_good_one_is_deleted(pool: PgPool) {
        queue(&pool).await;
        let from: Mailbox = "rmm@example.com".parse().unwrap();
        send_due(&pool, &AsyncStubTransport::new_error(), &from)
            .await
            .unwrap();
        let (attempts, later): (i32, bool) =
            sqlx::query_as("SELECT attempts, next_attempt_at > now() FROM email_outbox")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((attempts, later), (1, true));

        // Not due yet: nothing is sent.
        let ok = AsyncStubTransport::new_ok();
        send_due(&pool, &ok, &from).await.unwrap();
        assert!(ok.messages().await.is_empty());

        sqlx::query("UPDATE email_outbox SET next_attempt_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        send_due(&pool, &ok, &from).await.unwrap();
        assert_eq!(ok.messages().await.len(), 1);
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[sqlx::test]
    async fn the_last_failed_attempt_drops_the_email(pool: PgPool) {
        queue(&pool).await;
        sqlx::query("UPDATE email_outbox SET attempts = 3")
            .execute(&pool)
            .await
            .unwrap();
        let from: Mailbox = "rmm@example.com".parse().unwrap();
        send_due(&pool, &AsyncStubTransport::new_error(), &from)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[sqlx::test]
    async fn a_failed_record_does_not_resend_earlier_emails(pool: PgPool) {
        let op = queue(&pool).await;
        sqlx::query("DELETE FROM email_outbox")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO email_outbox (id, operator_id, subject, body, next_attempt_at) VALUES
             ($1, $3, 'first', 'b', now() - interval '2 minutes'),
             ($2, $3, 'second', 'b', now() - interval '1 minute')",
        )
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(
            "CREATE FUNCTION fail_second() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 IF OLD.subject = 'second' THEN RAISE EXCEPTION 'injected failure'; END IF;
                 RETURN OLD;
             END $$;
             CREATE TRIGGER fail_second BEFORE DELETE ON email_outbox
                 FOR EACH ROW EXECUTE FUNCTION fail_second();",
        )
        .execute(&pool)
        .await
        .unwrap();
        let from: Mailbox = "rmm@example.com".parse().unwrap();
        let ok = AsyncStubTransport::new_ok();
        assert!(send_due(&pool, &ok, &from).await.is_err());
        let sent = ok.messages().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[0].1.contains("Subject: first"));
        let left: Vec<(String, bool)> =
            sqlx::query_as("SELECT subject, next_attempt_at > now() FROM email_outbox")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(left, vec![("second".to_string(), true)]);

        // Leased: the next run sends nothing.
        let ok = AsyncStubTransport::new_ok();
        send_due(&pool, &ok, &from).await.unwrap();
        assert!(ok.messages().await.is_empty());

        sqlx::raw_sql("DROP TRIGGER fail_second ON email_outbox")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE email_outbox SET next_attempt_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        let ok = AsyncStubTransport::new_ok();
        send_due(&pool, &ok, &from).await.unwrap();
        let sent = ok.messages().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.contains("Subject: second"));
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }
}
