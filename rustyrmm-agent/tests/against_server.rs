//! The Agent against the real Server (in-process, real mTLS, real Postgres).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustyrmm_agent::session::{self, renew_if_due};
use rustyrmm_agent::state::Paired;
use rustyrmm_agent::{pairing, tls};
use rustyrmm_proto::agent::v1::agent_service_client::AgentServiceClient;
use rustyrmm_proto::agent::v1::agent_service_server::AgentServiceServer;
use rustyrmm_server::agent_service::{tls_config, AgentApi};
use rustyrmm_server::ca::Ca;
use rustyrmm_server::sessions::{Sessions, Settings};
use sqlx::PgPool;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

struct Server {
    pool: PgPool,
    addr: SocketAddr,
    fingerprint: String,
    sessions: Arc<Sessions>,
    environment_id: Uuid,
    _dir: tempfile::TempDir,
}

async fn start(pool: PgPool, settings: Settings) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let ca = Arc::new(Ca::load_or_create(dir.path()).unwrap());
    let tls = tls_config(&ca, vec!["127.0.0.1".into()]).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let sessions = Arc::new(Sessions::default());
    let fingerprint = ca.fingerprint_sha256();
    tokio::spawn(
        tonic::transport::Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(AgentServiceServer::new(AgentApi {
                pool: pool.clone(),
                ca,
                sessions: sessions.clone(),
                settings,
            }))
            .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener)),
    );
    let environment_id = Uuid::now_v7();
    sqlx::query("INSERT INTO environments (id, name) VALUES ($1, 'test')")
        .bind(environment_id)
        .execute(&pool)
        .await
        .unwrap();
    Server {
        pool,
        addr,
        fingerprint,
        sessions,
        environment_id,
        _dir: dir,
    }
}

impl Server {
    async fn token(&self) -> String {
        rustyrmm_server::pairing::create_token(
            &self.pool,
            self.environment_id,
            chrono::Duration::days(7),
        )
        .await
        .unwrap()
        .1
    }

    /// Pairs a new Agent state directory; returns (dir, Endpoint id).
    async fn pair(&self) -> (tempfile::TempDir, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let id = pairing::pair(
            dir.path(),
            &self.addr.to_string(),
            &self.token().await,
            &self.fingerprint,
        )
        .await
        .unwrap();
        (dir, id.parse().unwrap())
    }
}

async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        sleep(Duration::from_millis(20)).await;
    }
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn pairs_holds_a_session_and_reconnects(pool: PgPool) {
    let server = start(pool, Settings::default()).await;
    let (dir, ep) = server.pair().await;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("agent.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the Agent key must be owner-only");
    }

    let path = dir.path().to_path_buf();
    let agent = tokio::spawn(async move { session::run(&path).await });
    eventually("Endpoint Online", || server.sessions.is_online(ep)).await;

    // The Server drops the Session; the Agent reconnects with backoff.
    server.sessions.disconnect(ep);
    assert!(!server.sessions.is_online(ep));
    eventually("Endpoint Online again", || server.sessions.is_online(ep)).await;
    agent.abort();
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn pair_refuses_a_server_whose_ca_does_not_match_the_pin(pool: PgPool) {
    let server = start(pool, Settings::default()).await;
    let other_ca = Ca::load_or_create(tempfile::tempdir().unwrap().path()).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let err = pairing::pair(
        dir.path(),
        &server.addr.to_string(),
        &server.token().await,
        &other_ca.fingerprint_sha256(),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").to_lowercase().contains("transport"),
        "{err:#}"
    );
    assert!(!dir.path().join("agent.key").exists(), "nothing saved");
    let endpoints: i64 = sqlx::query_scalar("SELECT count(*) FROM endpoints")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(endpoints, 0, "the token never reached the Server");
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn heartbeats_keep_the_session_alive(pool: PgPool) {
    let settings = Settings {
        heartbeat_interval: Duration::from_millis(200),
        offline_after: Duration::from_millis(1000),
        ..Settings::default()
    };
    let server = start(pool, settings).await;
    let (dir, ep) = server.pair().await;
    let paired = Paired::load(dir.path()).unwrap();

    let outbox = Arc::new(rustyrmm_agent::commands::Outbox::default());
    let session = tokio::spawn(async move { session::session_once(&paired, &outbox).await });
    eventually("Endpoint Online", || server.sessions.is_online(ep)).await;
    // Three times the offline timeout: without heartbeats the Server would have closed it.
    sleep(Duration::from_secs(3)).await;
    assert!(!session.is_finished(), "Session ended: {:?}", session.await);
    assert!(server.sessions.is_online(ep));
    session.abort();
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn renews_the_certificate_only_when_due(pool: PgPool) {
    let server = start(pool, Settings::default()).await;
    let (dir, _) = server.pair().await;
    let paired = Paired::load(dir.path()).unwrap();
    let mut client = AgentServiceClient::new(session::connect(&paired).await.unwrap());

    let day = Duration::from_secs(86_400);
    assert!(!renew_if_due(&mut client, &paired, day).await.unwrap());
    assert!(renew_if_due(&mut client, &paired, 400 * day).await.unwrap());

    let renewed = Paired::load(dir.path()).unwrap();
    assert_ne!(renewed.cert_pem, paired.cert_pem);
    // The renewed certificate works for a new connection.
    session::connect(&renewed).await.unwrap();
    let certs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_certificates")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(certs, 2);
}

#[test]
fn verifier_rejects_garbage_fingerprint() {
    assert!(tls::PinnedCaVerifier::new("zz").is_err());
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn sends_inventory_and_samples_the_server_stores(pool: PgPool) {
    let settings = Settings {
        sample_interval: Duration::from_millis(300),
        ..Settings::default()
    };
    let server = start(pool, settings).await;
    let (dir, ep) = server.pair().await;
    let path = dir.path().to_path_buf();
    let agent = tokio::spawn(async move { session::run(&path).await });

    // Inventory includes a real update check, which can take a while.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let (samples, inventories, hostname): (i64, i64, String) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM samples WHERE endpoint_id = $1),
                    (SELECT count(*) FROM inventories WHERE endpoint_id = $1),
                    (SELECT hostname FROM endpoints WHERE id = $1)",
        )
        .bind(ep)
        .fetch_one(&server.pool)
        .await
        .unwrap();
        if samples >= 2 && inventories == 1 && !hostname.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "samples={samples} inventories={inventories}"
        );
        sleep(Duration::from_millis(100)).await;
    }
    let memory_total: i64 =
        sqlx::query_scalar("SELECT memory_total_bytes FROM samples WHERE endpoint_id = $1 LIMIT 1")
            .bind(ep)
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert!(memory_total > 0);
    agent.abort();
}

// --- Commands and process lists ---

use rustyrmm_proto::common::v1::Interpreter;
use rustyrmm_server::commands::{dispatch, NewCommand};

fn sh(endpoint_id: Uuid, script: &str) -> NewCommand {
    NewCommand {
        endpoint_id,
        operator_username: "alice".into(),
        interpreter: Interpreter::Sh,
        script: script.into(),
        timeout_seconds: 30,
    }
}

/// Waits for the Command's stored (status, stdout).
async fn completed(pool: &PgPool, id: Uuid) -> (i16, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (status, stdout): (i16, Option<Vec<u8>>) =
            sqlx::query_as("SELECT status, stdout FROM commands WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        if status == 2 {
            return (status, stdout.unwrap_or_default());
        }
        assert!(Instant::now() < deadline, "Command never completed");
        sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn runs_commands_and_answers_process_lists(pool: PgPool) {
    let server = start(pool, Settings::default()).await;
    let (dir, ep) = server.pair().await;
    let path = dir.path().to_path_buf();
    let agent = tokio::spawn(async move { session::run(&path).await });
    eventually("Endpoint Online", || server.sessions.is_online(ep)).await;

    let id = dispatch(&server.pool, &server.sessions, sh(ep, "echo hello from $0"))
        .await
        .unwrap();
    let (_, stdout) = completed(&server.pool, id).await;
    assert!(String::from_utf8_lossy(&stdout).starts_with("hello from"));

    let processes = server
        .sessions
        .request_process_list(ep, Duration::from_secs(10))
        .await
        .unwrap();
    let me = std::process::id();
    assert!(
        processes.iter().any(|p| p.pid == me),
        "the test process is listed"
    );
    agent.abort();
}

#[cfg(unix)]
#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn result_finished_while_disconnected_is_resent(pool: PgPool) {
    let server = start(pool, Settings::default()).await;
    let (dir, ep) = server.pair().await;
    let path = dir.path().to_path_buf();
    let agent = tokio::spawn(async move { session::run(&path).await });
    eventually("Endpoint Online", || server.sessions.is_online(ep)).await;

    let id = dispatch(&server.pool, &server.sessions, sh(ep, "sleep 1; echo done"))
        .await
        .unwrap();
    // Drop the Session while the Command runs; the result must arrive on the next Session.
    sleep(Duration::from_millis(300)).await;
    server.sessions.disconnect(ep);
    let (_, stdout) = completed(&server.pool, id).await;
    assert_eq!(stdout, b"done\n");
    agent.abort();
}
