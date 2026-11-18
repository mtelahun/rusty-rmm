//! The Agent port's gRPC service.

use std::sync::Arc;

use rustyrmm_proto::agent::v1::agent_service_server::AgentService;
use rustyrmm_proto::agent::v1::{
    AgentMessage, PairRequest, PairResponse, RenewCertificateRequest, RenewCertificateResponse,
    ServerMessage,
};
use sqlx::PgPool;
use tonic::codegen::BoxStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::ca::{Ca, IssuedCert};
use crate::pairing;

pub struct AgentApi {
    pub pool: PgPool,
    pub ca: Arc<Ca>,
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
        _req: Request<Streaming<AgentMessage>>,
    ) -> Result<Response<Self::OpenSessionStream>, Status> {
        // shortcut: Sessions arrive with Task: Server Sessions and ingestion.
        Err(Status::unimplemented("Sessions are not implemented yet"))
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
        _dir: tempfile::TempDir,
    }

    /// Runs the Agent port with real TLS on a random local port.
    async fn start(pool: PgPool) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let ca = Arc::new(Ca::load_or_create(dir.path()).unwrap());
        let tls = tls_config(&ca, vec!["localhost".into()]).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ca_pem = ca.cert_pem().to_owned();
        let api = AgentApi {
            pool: pool.clone(),
            ca,
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
}
