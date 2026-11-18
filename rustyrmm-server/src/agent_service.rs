//! The Agent port's gRPC service.

use std::sync::Arc;

use rustyrmm_proto::agent::v1::agent_service_server::AgentService;
use rustyrmm_proto::agent::v1::{
    agent_message, server_message, AgentMessage, HelloAck, PairRequest, PairResponse,
    RenewCertificateRequest, RenewCertificateResponse, ServerMessage,
};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};
use tokio_stream::wrappers::ReceiverStream;
use tonic::codegen::BoxStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::ca::{Ca, IssuedCert};
use crate::sessions::{Registration, Sessions, Settings};
use crate::{commands, ingest, pairing};

/// Agents older than this protocol version are refused at Hello.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

pub struct AgentApi {
    pub pool: PgPool,
    pub ca: Arc<Ca>,
    pub sessions: Arc<Sessions>,
    pub settings: Settings,
}

#[tonic::async_trait]
impl AgentService for AgentApi {
    async fn pair(&self, req: Request<PairRequest>) -> Result<Response<PairResponse>, Status> {
        let req = req.into_inner();
        let environment_id = pairing::redeem(&self.pool, &req.pairing_token)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                Status::permission_denied("Pairing Token is invalid, expired or revoked")
            })?;

        let endpoint_id = Uuid::now_v7();
        let issued = self.sign(&req.csr_pem, endpoint_id)?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        // Hostname is filled in from the Endpoint's first Inventory.
        sqlx::query("INSERT INTO endpoints (id, environment_id, hostname) VALUES ($1, $2, '')")
            .bind(endpoint_id)
            .bind(environment_id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        record_cert(&mut tx, endpoint_id, &issued).await?;
        tx.commit().await.map_err(internal)?;

        Ok(Response::new(PairResponse {
            endpoint_id: endpoint_id.to_string(),
            certificate_pem: issued.pem,
        }))
    }

    async fn renew_certificate(
        &self,
        req: Request<RenewCertificateRequest>,
    ) -> Result<Response<RenewCertificateResponse>, Status> {
        let endpoint_id = authenticated_endpoint(&self.pool, &req).await?;
        let issued = self.sign(&req.into_inner().csr_pem, endpoint_id)?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        record_cert(&mut tx, endpoint_id, &issued).await?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(RenewCertificateResponse {
            certificate_pem: issued.pem,
        }))
    }

    type OpenSessionStream = BoxStream<ServerMessage>;

    async fn open_session(
        &self,
        req: Request<Streaming<AgentMessage>>,
    ) -> Result<Response<Self::OpenSessionStream>, Status> {
        let endpoint_id = authenticated_endpoint(&self.pool, &req).await?;
        let mut inbound = req.into_inner();

        let first = timeout(self.settings.offline_after, inbound.message()).await;
        let hello = match first {
            Ok(Ok(Some(AgentMessage {
                message: Some(agent_message::Message::Hello(hello)),
            }))) => hello,
            _ => {
                return Err(Status::failed_precondition(
                    "the first message must be Hello",
                ))
            }
        };
        if hello.protocol_version < MIN_PROTOCOL_VERSION {
            return Err(Status::failed_precondition(format!(
                "protocol version {} is below the minimum {MIN_PROTOCOL_VERSION}; upgrade the Agent",
                hello.protocol_version
            )));
        }

        let (outbound, rx) = mpsc::channel(16);
        let s = self.settings;
        let ack = HelloAck {
            inventory_interval: Some(proto_duration(s.inventory_interval)),
            sample_interval: Some(proto_duration(s.sample_interval)),
            heartbeat_interval: Some(proto_duration(s.heartbeat_interval)),
        };
        outbound
            .send(Ok(ServerMessage {
                message: Some(server_message::Message::HelloAck(ack)),
            }))
            .await
            .map_err(internal)?;
        ingest::touch_last_seen(&self.pool, endpoint_id)
            .await
            .map_err(internal)?;

        // The registry holds the only sender: when the Session is removed or replaced, the
        // outbound stream ends and the Agent sees its Session close.
        let registration = self.sessions.register(endpoint_id, outbound);
        tokio::spawn(run_session(
            self.pool.clone(),
            self.sessions.clone(),
            s,
            endpoint_id,
            registration,
            inbound,
        ));
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Reads one Session's messages until the Agent disconnects, goes silent past `offline_after`,
/// or the Session is replaced or disconnected.
async fn run_session(
    pool: PgPool,
    sessions: Arc<Sessions>,
    settings: Settings,
    endpoint_id: Uuid,
    registration: Registration,
    mut inbound: Streaming<AgentMessage>,
) {
    let mut last_seen_written = Instant::now();
    loop {
        let msg = tokio::select! {
            _ = registration.close.notified() => break,
            msg = timeout(settings.offline_after, inbound.message()) => msg,
        };
        let msg = match msg {
            Ok(Ok(Some(msg))) => msg,
            // Silent too long, stream error, or clean disconnect.
            _ => break,
        };
        if last_seen_written.elapsed() >= settings.last_seen_write_interval {
            last_seen_written = Instant::now();
            log_err(ingest::touch_last_seen(&pool, endpoint_id).await);
        }
        match msg.message {
            Some(agent_message::Message::Inventory(inv)) => {
                log_err(ingest::inventory(&pool, endpoint_id, &inv).await.map(drop))
            }
            Some(agent_message::Message::Sample(sample)) => {
                log_err(ingest::sample(&pool, endpoint_id, &sample).await)
            }
            Some(agent_message::Message::CommandResult(result)) => {
                log_err(commands::record_result(&pool, endpoint_id, &result).await)
            }
            Some(agent_message::Message::ProcessList(list)) => {
                sessions.complete_process_list(endpoint_id, list)
            }
            // Heartbeats only keep the Session alive; a repeated Hello is ignored.
            Some(agent_message::Message::Heartbeat(_))
            | Some(agent_message::Message::Hello(_))
            | None => {}
        }
    }
    sessions.remove(endpoint_id, registration.id);
    log_err(ingest::touch_last_seen(&pool, endpoint_id).await);
}

fn proto_duration(d: std::time::Duration) -> prost_types::Duration {
    prost_types::Duration {
        seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        nanos: d.subsec_nanos() as i32,
    }
}

fn log_err(r: sqlx::Result<()>) {
    if let Err(e) = r {
        // shortcut: stderr until the Server adopts structured logging.
        eprintln!("session storage error: {e}");
    }
}

impl AgentApi {
    fn sign(&self, csr_pem: &str, endpoint_id: Uuid) -> Result<IssuedCert, Status> {
        self.ca.sign_agent_csr(csr_pem, endpoint_id).map_err(|e| {
            Status::invalid_argument(format!("invalid certificate signing request: {e}"))
        })
    }
}

/// Identifies the calling Endpoint from its client certificate. TLS has already verified the
/// chain to our CA and its validity dates; this also requires that the certificate was issued
/// to a current, non-Revoked Endpoint (so certificates of deleted Endpoints stop working).
pub async fn authenticated_endpoint<T>(pool: &PgPool, req: &Request<T>) -> Result<Uuid, Status> {
    let unauthenticated = || Status::unauthenticated("a valid Agent certificate is required");
    let certs = req.peer_certs().ok_or_else(unauthenticated)?;
    let leaf = certs.first().ok_or_else(unauthenticated)?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf).map_err(|_| unauthenticated())?;
    let endpoint_id: Uuid = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .and_then(|cn| cn.parse().ok())
        .ok_or_else(unauthenticated)?;

    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM agent_certificates c JOIN endpoints e ON e.id = c.endpoint_id
             WHERE c.serial = $1 AND c.endpoint_id = $2 AND e.revoked_at IS NULL)",
    )
    .bind(cert.raw_serial())
    .bind(endpoint_id)
    .fetch_one(pool)
    .await
    .map_err(internal)?;
    if active {
        Ok(endpoint_id)
    } else {
        Err(unauthenticated())
    }
}

async fn record_cert(
    tx: &mut sqlx::PgConnection,
    endpoint_id: Uuid,
    issued: &IssuedCert,
) -> Result<(), Status> {
    sqlx::query(
        "INSERT INTO agent_certificates (serial, endpoint_id, not_before, not_after)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&issued.serial)
    .bind(endpoint_id)
    .bind(to_chrono(issued.not_before))
    .bind(to_chrono(issued.not_after))
    .execute(tx)
    .await
    .map_err(internal)?;
    Ok(())
}

fn to_chrono(t: time::OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(t.unix_timestamp(), 0).expect("certificate dates are in range")
}

fn internal(e: impl std::fmt::Display) -> Status {
    // shortcut: logs to stderr; switch to structured logging when the Server gets tracing.
    eprintln!("internal error: {e}");
    Status::internal("internal error")
}

/// TLS for the Agent port: Server identity from our CA; client certificates optional at the TLS
/// layer (so `Pair` works without one) and enforced per call by `authenticated_endpoint`.
pub fn tls_config(
    ca: &Ca,
    agent_names: Vec<String>,
) -> anyhow::Result<tonic::transport::ServerTlsConfig> {
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};
    let (chain, key) = ca.agent_port_identity(agent_names)?;
    Ok(ServerTlsConfig::new()
        .identity(Identity::from_pem(chain, key))
        .client_ca_root(Certificate::from_pem(ca.cert_pem()))
        .client_auth_optional(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use rustyrmm_proto::agent::v1::agent_service_client::AgentServiceClient;
    use rustyrmm_proto::agent::v1::agent_service_server::AgentServiceServer;
    use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

    struct Harness {
        pool: PgPool,
        addr: std::net::SocketAddr,
        ca_pem: String,
        environment_id: Uuid,
        sessions: Arc<Sessions>,
        _dir: tempfile::TempDir,
    }

    async fn start(pool: PgPool) -> Harness {
        start_with(pool, Settings::default()).await
    }

    /// Runs the Agent port with real TLS on a random local port.
    async fn start_with(pool: PgPool, settings: Settings) -> Harness {
        let sessions = Arc::new(Sessions::default());
        let dir = tempfile::tempdir().unwrap();
        let ca = Arc::new(Ca::load_or_create(dir.path()).unwrap());
        let tls = tls_config(&ca, vec!["localhost".into()]).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ca_pem = ca.cert_pem().to_owned();
        let api = AgentApi {
            pool: pool.clone(),
            ca,
            sessions: sessions.clone(),
            settings,
        };
        tokio::spawn(
            tonic::transport::Server::builder()
                .tls_config(tls)
                .unwrap()
                .add_service(AgentServiceServer::new(api))
                .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener)),
        );
        let environment_id = Uuid::now_v7();
        sqlx::query("INSERT INTO environments (id, name) VALUES ($1, 'test')")
            .bind(environment_id)
            .execute(&pool)
            .await
            .unwrap();
        Harness {
            pool,
            addr,
            ca_pem,
            environment_id,
            sessions,
            _dir: dir,
        }
    }

    impl Harness {
        async fn client(&self, identity: Option<Identity>) -> AgentServiceClient<Channel> {
            let mut tls = ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&self.ca_pem))
                .domain_name("localhost");
            if let Some(id) = identity {
                tls = tls.identity(id);
            }
            let channel = Channel::from_shared(format!("https://{}", self.addr))
                .unwrap()
                .tls_config(tls)
                .unwrap()
                .connect()
                .await
                .unwrap();
            AgentServiceClient::new(channel)
        }

        async fn token(&self, ttl: chrono::Duration) -> (Uuid, String) {
            pairing::create_token(&self.pool, self.environment_id, ttl)
                .await
                .unwrap()
        }
    }

    fn csr(key: &KeyPair) -> String {
        CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .serialize_request(key)
            .unwrap()
            .pem()
            .unwrap()
    }

    async fn pair(h: &Harness, key: &KeyPair) -> (String, Identity) {
        let (_, secret) = h.token(chrono::Duration::days(7)).await;
        let resp = h
            .client(None)
            .await
            .pair(PairRequest {
                pairing_token: secret,
                csr_pem: csr(key),
            })
            .await
            .unwrap()
            .into_inner();
        let identity = Identity::from_pem(&resp.certificate_pem, key.serialize_pem());
        (resp.endpoint_id, identity)
    }

    #[sqlx::test]
    async fn pairs_then_renews_with_the_issued_certificate(pool: PgPool) {
        let h = start(pool).await;
        let key = KeyPair::generate().unwrap();
        let (endpoint_id, identity) = pair(&h, &key).await;

        let env: Uuid = sqlx::query_scalar("SELECT environment_id FROM endpoints WHERE id = $1")
            .bind(endpoint_id.parse::<Uuid>().unwrap())
            .fetch_one(&h.pool)
            .await
            .unwrap();
        assert_eq!(env, h.environment_id);

        let renewed = h
            .client(Some(identity))
            .await
            .renew_certificate(RenewCertificateRequest { csr_pem: csr(&key) })
            .await
            .unwrap()
            .into_inner();
        assert!(renewed.certificate_pem.contains("BEGIN CERTIFICATE"));
        let certs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_certificates")
            .fetch_one(&h.pool)
            .await
            .unwrap();
        assert_eq!(certs, 2);
    }

    #[sqlx::test]
    async fn pair_refuses_invalid_expired_and_revoked_tokens(pool: PgPool) {
        let h = start(pool).await;
        let key = KeyPair::generate().unwrap();
        let (_, expired) = h.token(chrono::Duration::seconds(-1)).await;
        let (revoked_id, revoked) = h.token(chrono::Duration::days(7)).await;
        sqlx::query("UPDATE pairing_tokens SET revoked_at = now() WHERE id = $1")
            .bind(revoked_id)
            .execute(&h.pool)
            .await
            .unwrap();

        for secret in [expired, revoked, "not-a-token".to_string()] {
            let err = h
                .client(None)
                .await
                .pair(PairRequest {
                    pairing_token: secret,
                    csr_pem: csr(&key),
                })
                .await
                .unwrap_err();
            assert_eq!(err.code(), tonic::Code::PermissionDenied);
        }
    }

    #[sqlx::test]
    async fn pair_rejects_a_malformed_csr(pool: PgPool) {
        let h = start(pool).await;
        let (_, secret) = h.token(chrono::Duration::days(7)).await;
        let err = h
            .client(None)
            .await
            .pair(PairRequest {
                pairing_token: secret,
                csr_pem: "garbage".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    async fn renew(mut client: AgentServiceClient<Channel>) -> tonic::Code {
        let key = KeyPair::generate().unwrap();
        client
            .renew_certificate(RenewCertificateRequest { csr_pem: csr(&key) })
            .await
            .unwrap_err()
            .code()
    }

    #[sqlx::test]
    async fn renew_requires_a_certificate_of_a_current_endpoint(pool: PgPool) {
        let h = start(pool).await;

        // No client certificate at all.
        assert_eq!(
            renew(h.client(None).await).await,
            tonic::Code::Unauthenticated
        );

        // Revoked Endpoint.
        let (endpoint_id, identity) = pair(&h, &KeyPair::generate().unwrap()).await;
        sqlx::query("UPDATE endpoints SET revoked_at = now() WHERE id = $1")
            .bind(endpoint_id.parse::<Uuid>().unwrap())
            .execute(&h.pool)
            .await
            .unwrap();
        assert_eq!(
            renew(h.client(Some(identity)).await).await,
            tonic::Code::Unauthenticated
        );

        // Deleted Endpoint: its certificate is still CA-signed but no longer on record.
        let (endpoint_id, identity) = pair(&h, &KeyPair::generate().unwrap()).await;
        sqlx::query("DELETE FROM endpoints WHERE id = $1")
            .bind(endpoint_id.parse::<Uuid>().unwrap())
            .execute(&h.pool)
            .await
            .unwrap();
        assert_eq!(
            renew(h.client(Some(identity)).await).await,
            tonic::Code::Unauthenticated
        );
    }

    // --- Sessions ---

    use rustyrmm_proto::agent::v1::{Heartbeat, Hello};
    use rustyrmm_proto::common::v1::{InterfaceSample, Inventory, Sample};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    fn msg(m: agent_message::Message) -> AgentMessage {
        AgentMessage { message: Some(m) }
    }

    fn hello(protocol_version: u32) -> AgentMessage {
        msg(agent_message::Message::Hello(Hello {
            agent_version: "test".into(),
            protocol_version,
        }))
    }

    /// Opens a Session whose first message is `first`. Returns the Agent's sender and the
    /// Server's reply stream (or the error status).
    async fn open(
        h: &Harness,
        identity: Identity,
        first: AgentMessage,
    ) -> (
        mpsc::Sender<AgentMessage>,
        Result<Streaming<ServerMessage>, tonic::Status>,
    ) {
        let (tx, rx) = mpsc::channel(16);
        tx.send(first).await.unwrap();
        let resp = h
            .client(Some(identity))
            .await
            .open_session(ReceiverStream::new(rx))
            .await
            .map(|r| r.into_inner());
        (tx, resp)
    }

    async fn paired_endpoint(h: &Harness) -> (Uuid, Identity) {
        let (id, identity) = pair(h, &KeyPair::generate().unwrap()).await;
        (id.parse().unwrap(), identity)
    }

    /// Polls until `cond` holds or WAIT elapses.
    async fn eventually(mut cond: impl AsyncFnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !cond().await {
            assert!(Instant::now() < deadline, "condition not met in time");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn stream_ends(stream: &mut Streaming<ServerMessage>) {
        let next = timeout(WAIT, stream.message())
            .await
            .expect("stream should end");
        assert!(
            !matches!(next, Ok(Some(_))),
            "expected end of stream, got {next:?}"
        );
    }

    #[sqlx::test]
    async fn session_acks_hello_goes_online_and_stores_data(pool: PgPool) {
        let h = start(pool).await;
        let (ep, identity) = paired_endpoint(&h).await;
        let (tx, resp) = open(&h, identity, hello(1)).await;
        let mut stream = resp.unwrap();

        let ack = stream.message().await.unwrap().unwrap();
        let Some(server_message::Message::HelloAck(ack)) = ack.message else {
            panic!("expected HelloAck, got {ack:?}");
        };
        assert_eq!(ack.sample_interval.unwrap().seconds, 30);
        assert_eq!(ack.heartbeat_interval.unwrap().seconds, 30);
        assert_eq!(ack.inventory_interval.unwrap().seconds, 3600);
        assert!(h.sessions.is_online(ep));

        let inv = Inventory {
            hostname: "web-01".into(),
            ..Default::default()
        };
        for _ in 0..2 {
            // The second, identical Inventory must not add a row.
            tx.send(msg(agent_message::Message::Inventory(inv.clone())))
                .await
                .unwrap();
        }
        tx.send(msg(agent_message::Message::Heartbeat(Heartbeat {})))
            .await
            .unwrap();
        tx.send(msg(agent_message::Message::Sample(Sample {
            cpu_usage_percent: 12.5,
            memory_used_bytes: 1024,
            interfaces: vec![InterfaceSample {
                name: "eth0".into(),
                link_speed_bps: None,
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await
        .unwrap();

        let pool = &h.pool;
        eventually(async || {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM samples")
                .fetch_one(pool)
                .await
                .unwrap();
            n == 1
        })
        .await;
        let (hostname, last_seen): (String, Option<chrono::DateTime<chrono::Utc>>) =
            sqlx::query_as("SELECT hostname, last_seen_at FROM endpoints WHERE id = $1")
                .bind(ep)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(hostname, "web-01");
        assert!(last_seen.is_some());
        let inventories: i64 = sqlx::query_scalar("SELECT count(*) FROM inventories")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(inventories, 1);
        let link_speed: serde_json::Value =
            sqlx::query_scalar("SELECT interfaces -> 0 -> 'link_speed_bps' FROM samples")
                .fetch_one(pool)
                .await
                .unwrap();
        assert!(
            link_speed.is_null(),
            "unknown link speed must be null, not 0"
        );

        drop(tx);
        eventually(async || !h.sessions.is_online(ep)).await;
    }

    #[sqlx::test]
    async fn session_requires_hello_first_and_a_supported_protocol(pool: PgPool) {
        let h = start(pool).await;
        let (_, identity) = paired_endpoint(&h).await;
        let (_tx, resp) = open(
            &h,
            identity.clone(),
            msg(agent_message::Message::Heartbeat(Heartbeat {})),
        )
        .await;
        assert_eq!(resp.unwrap_err().code(), tonic::Code::FailedPrecondition);

        let (_tx, resp) = open(&h, identity, hello(0)).await;
        assert_eq!(resp.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }

    #[sqlx::test]
    async fn newer_session_replaces_the_old_one(pool: PgPool) {
        let h = start(pool).await;
        let (ep, identity) = paired_endpoint(&h).await;
        let (_tx1, resp1) = open(&h, identity.clone(), hello(1)).await;
        let mut first = resp1.unwrap();
        first.message().await.unwrap(); // HelloAck

        let (_tx2, resp2) = open(&h, identity, hello(1)).await;
        let mut second = resp2.unwrap();
        second.message().await.unwrap(); // HelloAck

        stream_ends(&mut first).await;
        assert!(h.sessions.is_online(ep));
    }

    #[sqlx::test]
    async fn silent_session_goes_offline(pool: PgPool) {
        let settings = Settings {
            offline_after: Duration::from_millis(300),
            ..Settings::default()
        };
        let h = start_with(pool, settings).await;
        let (ep, identity) = paired_endpoint(&h).await;
        let (_tx, resp) = open(&h, identity, hello(1)).await;
        let mut stream = resp.unwrap();
        stream.message().await.unwrap(); // HelloAck
        assert!(h.sessions.is_online(ep));

        stream_ends(&mut stream).await;
        assert!(!h.sessions.is_online(ep));
    }

    #[sqlx::test]
    async fn revoked_endpoint_is_disconnected_and_cannot_reconnect(pool: PgPool) {
        let h = start(pool).await;
        let (ep, identity) = paired_endpoint(&h).await;
        let (_tx, resp) = open(&h, identity.clone(), hello(1)).await;
        let mut stream = resp.unwrap();
        stream.message().await.unwrap(); // HelloAck

        // What RevokeEndpoint will do: mark Revoked, then close the live Session.
        sqlx::query("UPDATE endpoints SET revoked_at = now() WHERE id = $1")
            .bind(ep)
            .execute(&h.pool)
            .await
            .unwrap();
        h.sessions.disconnect(ep);
        stream_ends(&mut stream).await;

        let (_tx, resp) = open(&h, identity, hello(1)).await;
        assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    // --- Commands and process lists ---

    use crate::commands::{dispatch, DispatchError, NewCommand};
    use crate::sessions::LiveError;
    use rustyrmm_proto::agent::v1::ProcessList;
    use rustyrmm_proto::common::v1::{CommandResult, Interpreter, ProcessInfo};

    /// A paired Endpoint with an open Session, past the HelloAck.
    async fn online(h: &Harness) -> (Uuid, mpsc::Sender<AgentMessage>, Streaming<ServerMessage>) {
        let (ep, identity) = paired_endpoint(h).await;
        let (tx, resp) = open(h, identity, hello(1)).await;
        let mut stream = resp.unwrap();
        stream.message().await.unwrap(); // HelloAck
        (ep, tx, stream)
    }

    fn new_command(endpoint_id: Uuid) -> NewCommand {
        NewCommand {
            endpoint_id,
            operator_username: "alice".into(),
            interpreter: Interpreter::Sh,
            script: "echo hi".into(),
            timeout_seconds: 30,
        }
    }

    fn result(id: &str, stdout: &str) -> AgentMessage {
        msg(agent_message::Message::CommandResult(CommandResult {
            id: id.into(),
            exit_code: Some(0),
            stdout: stdout.into(),
            ..Default::default()
        }))
    }

    /// Waits until the Server has processed everything `tx` sent so far (messages are handled
    /// in order, so once this Sample is stored, earlier messages were handled too).
    async fn flush(h: &Harness, tx: &mpsc::Sender<AgentMessage>) {
        let pool = &h.pool;
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM samples")
            .fetch_one(pool)
            .await
            .unwrap();
        tx.send(msg(agent_message::Message::Sample(Sample::default())))
            .await
            .unwrap();
        eventually(async || {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM samples")
                .fetch_one(pool)
                .await
                .unwrap();
            n > before
        })
        .await;
    }

    #[sqlx::test]
    async fn command_round_trips_and_its_result_is_stored_once(pool: PgPool) {
        let h = start(pool).await;
        let (ep, tx, mut stream) = online(&h).await;
        let (_other, other_tx, _other_stream) = online(&h).await;

        let id = dispatch(&h.pool, &h.sessions, new_command(ep))
            .await
            .unwrap();
        let sent = stream.message().await.unwrap().unwrap();
        let Some(server_message::Message::Command(cmd)) = sent.message else {
            panic!("expected Command, got {sent:?}");
        };
        assert_eq!(cmd.id, id.to_string());
        assert_eq!(cmd.script, "echo hi");

        // Another Endpoint cannot complete this Command.
        other_tx.send(result(&cmd.id, "forged")).await.unwrap();
        // The real result, then a resend after a "reconnect" that must be ignored.
        tx.send(result(&cmd.id, "hi")).await.unwrap();
        tx.send(result(&cmd.id, "again")).await.unwrap();
        flush(&h, &other_tx).await;
        flush(&h, &tx).await;

        let (status, stdout, hostname): (i16, Vec<u8>, String) =
            sqlx::query_as("SELECT status, stdout, endpoint_hostname FROM commands WHERE id = $1")
                .bind(id)
                .fetch_one(&h.pool)
                .await
                .unwrap();
        assert_eq!(status, 2, "Completed");
        assert_eq!(stdout, b"hi");
        assert_eq!(hostname, "");
    }

    #[sqlx::test]
    async fn command_to_offline_endpoint_is_rejected_and_not_stored(pool: PgPool) {
        let h = start(pool).await;
        let (ep, _) = paired_endpoint(&h).await;
        let err = dispatch(&h.pool, &h.sessions, new_command(ep))
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::Live(LiveError::Offline)));
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(&h.pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[sqlx::test]
    async fn process_list_is_relayed_live(pool: PgPool) {
        let h = start(pool).await;
        let (ep, tx, mut stream) = online(&h).await;

        let sessions = h.sessions.clone();
        let request = tokio::spawn(async move {
            sessions
                .request_process_list(ep, Duration::from_secs(5))
                .await
        });
        let sent = stream.message().await.unwrap().unwrap();
        let Some(server_message::Message::ProcessListRequest(req)) = sent.message else {
            panic!("expected ProcessListRequest, got {sent:?}");
        };
        tx.send(msg(agent_message::Message::ProcessList(ProcessList {
            request_id: req.request_id,
            processes: vec![ProcessInfo {
                pid: 42,
                name: "init".into(),
                ..Default::default()
            }],
        })))
        .await
        .unwrap();
        let processes = request.await.unwrap().unwrap();
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].pid, 42);

        // An Agent that never answers times out; an Offline Endpoint fails at once.
        let silent = h
            .sessions
            .request_process_list(ep, Duration::from_millis(200))
            .await;
        assert_eq!(silent.unwrap_err(), LiveError::Timeout);
        let (offline, _) = paired_endpoint(&h).await;
        let err = h
            .sessions
            .request_process_list(offline, Duration::from_secs(5))
            .await;
        assert_eq!(err.unwrap_err(), LiveError::Offline);
    }
}
