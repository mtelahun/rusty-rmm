//! `rustyrmm-agent run`: keep a Session open with the Server, reconnecting with backoff.

use std::path::Path;
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use rustyrmm_proto::agent::v1::agent_service_client::AgentServiceClient;
use rustyrmm_proto::agent::v1::{
    agent_message, server_message, AgentMessage, Heartbeat, Hello, RenewCertificateRequest,
};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, Instant, MissedTickBehavior};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use crate::pairing::host;
use crate::state::Paired;

/// Sent in Hello; the Server refuses Agents below its minimum.
pub const PROTOCOL_VERSION: u32 = 1;
/// Renew the certificate when it expires within this window.
pub const RENEW_WITHIN: Duration = Duration::from_secs(30 * 86_400);
const RENEW_CHECK_EVERY: Duration = Duration::from_secs(86_400);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// A Session that lasted this long resets the backoff.
const STABLE_SESSION: Duration = Duration::from_secs(60);

/// Runs forever, keeping a Session open. Returns only if the Agent is not paired.
pub async fn run(dir: &Path) -> anyhow::Result<()> {
    Paired::load(dir)?; // Fail fast if not paired.
    let mut backoff = BACKOFF_MIN;
    loop {
        let started = Instant::now();
        // Reload each time: the certificate may have been renewed.
        let result = match Paired::load(dir) {
            Ok(paired) => session_once(&paired).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => eprintln!("Session closed by the Server"),
            Err(e) => eprintln!("Session failed: {e:#}"),
        }
        if started.elapsed() >= STABLE_SESSION {
            backoff = BACKOFF_MIN;
        }
        sleep(jittered(backoff)).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Between half and all of `backoff`, so Agents reconnecting after an outage spread out.
fn jittered(backoff: Duration) -> Duration {
    backoff.mul_f64(0.5 + rand::random::<f64>() * 0.5)
}

pub async fn connect(paired: &Paired) -> anyhow::Result<Channel> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(&paired.ca_pem))
        .identity(Identity::from_pem(&paired.cert_pem, &paired.key_pem))
        .domain_name(host(&paired.config.server)?);
    Ok(
        Channel::from_shared(format!("https://{}", paired.config.server))?
            .tls_config(tls)?
            .connect()
            .await?,
    )
}

/// One Session: Hello, then heartbeats at the Server's interval, until the stream ends.
/// Returns Ok when the Server closes the Session cleanly.
pub async fn session_once(paired: &Paired) -> anyhow::Result<()> {
    let mut client = AgentServiceClient::new(connect(paired).await?);
    let (tx, rx) = mpsc::channel(16);
    tx.send(message(agent_message::Message::Hello(Hello {
        agent_version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: PROTOCOL_VERSION,
    })))
    .await?;
    let mut inbound = client
        .open_session(ReceiverStream::new(rx))
        .await?
        .into_inner();

    let ack = match inbound.message().await?.and_then(|m| m.message) {
        Some(server_message::Message::HelloAck(ack)) => ack,
        other => anyhow::bail!("expected HelloAck, got {other:?}"),
    };
    let heartbeat_every = ack
        .heartbeat_interval
        .and_then(|d| Duration::try_from(d).ok())
        .filter(|d| !d.is_zero())
        .unwrap_or(Duration::from_secs(30));

    let mut heartbeat = interval(heartbeat_every);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await; // The first tick is immediate; Hello just went out.
    let mut renew_check = interval(RENEW_CHECK_EVERY);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                tx.send(message(agent_message::Message::Heartbeat(Heartbeat {}))).await?;
            }
            _ = renew_check.tick() => {
                if let Err(e) = renew_if_due(&mut client, paired, RENEW_WITHIN).await {
                    eprintln!("certificate renewal failed (will retry): {e:#}");
                }
            }
            msg = inbound.message() => match msg? {
                None => return Ok(()),
                Some(msg) => match msg.message {
                    // shortcut: Commands and process lists are handled by
                    // Task: Agent Commands and process lists; until then they are ignored.
                    Some(server_message::Message::Command(_))
                    | Some(server_message::Message::ProcessListRequest(_))
                    | Some(server_message::Message::HelloAck(_))
                    | None => {}
                },
            },
        }
    }
}

/// Renews the certificate if it expires within `window`. The new certificate is saved and used
/// from the next connection; the current Session keeps running. Returns whether it renewed.
pub async fn renew_if_due(
    client: &mut AgentServiceClient<Channel>,
    paired: &Paired,
    window: Duration,
) -> anyhow::Result<bool> {
    if !expires_within(&paired.cert_pem, window)? {
        return Ok(false);
    }
    // Same key: renewal proves continued possession; the key never leaves the Endpoint.
    let key = KeyPair::from_pem(&paired.key_pem)?;
    let csr = CertificateParams::new(Vec::<String>::new())?
        .serialize_request(&key)?
        .pem()?;
    let renewed = client
        .renew_certificate(RenewCertificateRequest { csr_pem: csr })
        .await?
        .into_inner();
    paired.save_cert(&renewed.certificate_pem)?;
    Ok(true)
}

fn expires_within(cert_pem: &str, window: Duration) -> anyhow::Result<bool> {
    let der = pem::parse(cert_pem)?.into_contents();
    let (_, cert) = x509_parser::parse_x509_certificate(&der)?;
    let remaining = cert.validity().time_to_expiration();
    Ok(remaining.is_none_or(|left| left.unsigned_abs() < window))
}

fn message(m: agent_message::Message) -> AgentMessage {
    AgentMessage { message: Some(m) }
}
