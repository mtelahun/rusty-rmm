//! The Operator port's gRPC service (used by the CLI directly and the web UI via grpc-web).

use chrono::{DateTime, Utc};
use rustyrmm_proto::operator::v1::operator_service_server::OperatorService;
use rustyrmm_proto::operator::v1::*;
use sqlx::PgPool;
use tonic::metadata::MetadataValue;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::auth::{self, authenticate, internal};
use crate::operators::{self, AddOperatorError};
use crate::pairing;

pub const DEFAULT_PAIRING_TOKEN_TTL: chrono::Duration = chrono::Duration::days(7);
const DEFAULT_PAGE_SIZE: u32 = 100;
const MAX_PAGE_SIZE: u32 = 1000;

pub struct OperatorApi {
    pub pool: PgPool,
    /// What Agents dial, for the Pairing one-liner (`host:port`).
    pub agent_address: String,
    pub ca_fingerprint: String,
}

#[tonic::async_trait]
impl OperatorService for OperatorApi {
    async fn login(&self, req: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let web = auth::is_grpc_web(&req);
        let LoginRequest { username, password } = req.into_inner();
        let (token, expires_at) = auth::login(&self.pool, &username, &password).await?;
        let mut resp = Response::new(LoginResponse {
            session_token: token.clone(),
            expires_at: Some(ts(expires_at)),
        });
        if web {
            let cookie = auth::set_cookie(&token, auth::SESSION_TTL.num_seconds());
            resp.metadata_mut().insert("set-cookie", header(&cookie)?);
        }
        Ok(resp)
    }

    async fn logout(
        &self,
        req: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        let operator = authenticate(&self.pool, &req).await?;
        auth::logout(&self.pool, &operator).await?;
        let mut resp = Response::new(LogoutResponse {});
        if auth::is_grpc_web(&req) {
            resp.metadata_mut()
                .insert("set-cookie", header(&auth::set_cookie("", 0))?);
        }
        Ok(resp)
    }

    async fn add_operator(
        &self,
        req: Request<AddOperatorRequest>,
    ) -> Result<Response<AddOperatorResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let AddOperatorRequest { username, password } = req.into_inner();
        let id = operators::add_operator(&self.pool, &username, &password)
            .await
            .map_err(|e| match e {
                AddOperatorError::UsernameTaken(_) => Status::already_exists(e.to_string()),
                AddOperatorError::InvalidUsername | AddOperatorError::PasswordTooShort => {
                    Status::invalid_argument(e.to_string())
                }
                e => internal(e),
            })?;
        Ok(Response::new(AddOperatorResponse {
            operator_id: id.to_string(),
        }))
    }

    async fn create_environment(
        &self,
        req: Request<CreateEnvironmentRequest>,
    ) -> Result<Response<Environment>, Status> {
        authenticate(&self.pool, &req).await?;
        let name = req.into_inner().name.trim().to_owned();
        if name.is_empty() {
            return Err(Status::invalid_argument(
                "Environment name must not be empty",
            ));
        }
        let id = Uuid::now_v7();
        let created_at: DateTime<Utc> = sqlx::query_scalar(
            "INSERT INTO environments (id, name) VALUES ($1, $2) RETURNING created_at",
        )
        .bind(id)
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Status::already_exists(format!("an Environment named {name:?} already exists"))
            }
            e => internal(e),
        })?;
        Ok(Response::new(Environment {
            id: id.to_string(),
            name,
            created_at: Some(ts(created_at)),
        }))
    }

    async fn list_environments(
        &self,
        req: Request<ListEnvironmentsRequest>,
    ) -> Result<Response<ListEnvironmentsResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let page = Page::new(req.page_size, &req.page_token)?;
        let rows: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, name, created_at FROM environments
             WHERE $1::uuid IS NULL OR id > $1 ORDER BY id LIMIT $2",
        )
        .bind(page.after)
        .bind(page.fetch())
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let (rows, next_page_token) = page.split(rows, |r| r.0);
        Ok(Response::new(ListEnvironmentsResponse {
            environments: rows
                .into_iter()
                .map(|(id, name, created_at)| Environment {
                    id: id.to_string(),
                    name,
                    created_at: Some(ts(created_at)),
                })
                .collect(),
            next_page_token,
        }))
    }

    async fn create_pairing_token(
        &self,
        req: Request<CreatePairingTokenRequest>,
    ) -> Result<Response<CreatePairingTokenResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let environment_id = parse_id(&req.environment_id, "environment_id")?;
        let ttl = match req.ttl {
            None => DEFAULT_PAIRING_TOKEN_TTL,
            Some(d) => chrono::Duration::new(d.seconds, d.nanos.max(0) as u32)
                .filter(|d| *d > chrono::Duration::zero())
                .ok_or_else(|| Status::invalid_argument("ttl must be positive"))?,
        };
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM environments WHERE id = $1)")
                .bind(environment_id)
                .fetch_one(&self.pool)
                .await
                .map_err(internal)?;
        if !exists {
            return Err(Status::not_found("no such Environment"));
        }
        let (id, secret) = pairing::create_token(&self.pool, environment_id, ttl)
            .await
            .map_err(internal)?;
        let token = self.pairing_token(id).await?;
        Ok(Response::new(CreatePairingTokenResponse {
            pairing_token: Some(token),
            secret,
            agent_address: self.agent_address.clone(),
            ca_fingerprint_sha256: self.ca_fingerprint.clone(),
        }))
    }

    async fn list_pairing_tokens(
        &self,
        req: Request<ListPairingTokensRequest>,
    ) -> Result<Response<ListPairingTokensResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let environment_id = parse_id(&req.environment_id, "environment_id")?;
        let page = Page::new(req.page_size, &req.page_token)?;
        let rows: Vec<TokenRow> = sqlx::query_as(
            "SELECT id, environment_id, created_at, expires_at, revoked_at IS NOT NULL
             FROM pairing_tokens
             WHERE environment_id = $1 AND ($2::uuid IS NULL OR id > $2) ORDER BY id LIMIT $3",
        )
        .bind(environment_id)
        .bind(page.after)
        .bind(page.fetch())
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let (rows, next_page_token) = page.split(rows, |r| r.0);
        Ok(Response::new(ListPairingTokensResponse {
            pairing_tokens: rows.into_iter().map(token_from_row).collect(),
            next_page_token,
        }))
    }

    async fn revoke_pairing_token(
        &self,
        req: Request<RevokePairingTokenRequest>,
    ) -> Result<Response<RevokePairingTokenResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().id, "id")?;
        let found = sqlx::query(
            "UPDATE pairing_tokens SET revoked_at = coalesce(revoked_at, now()) WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(internal)?
        .rows_affected();
        if found == 0 {
            return Err(Status::not_found("no such Pairing Token"));
        }
        Ok(Response::new(RevokePairingTokenResponse {}))
    }

    // shortcut: the Endpoint, Monitoring and Command calls arrive with
    // Task: Operator Endpoint, Monitoring and Command calls.
    async fn list_endpoints(
        &self,
        _: Request<ListEndpointsRequest>,
    ) -> Result<Response<ListEndpointsResponse>, Status> {
        Err(not_yet())
    }
    async fn get_endpoint(
        &self,
        _: Request<GetEndpointRequest>,
    ) -> Result<Response<Endpoint>, Status> {
        Err(not_yet())
    }
    async fn revoke_endpoint(
        &self,
        _: Request<RevokeEndpointRequest>,
    ) -> Result<Response<RevokeEndpointResponse>, Status> {
        Err(not_yet())
    }
    async fn delete_endpoint(
        &self,
        _: Request<DeleteEndpointRequest>,
    ) -> Result<Response<DeleteEndpointResponse>, Status> {
        Err(not_yet())
    }
    async fn list_samples(
        &self,
        _: Request<ListSamplesRequest>,
    ) -> Result<Response<ListSamplesResponse>, Status> {
        Err(not_yet())
    }
    async fn get_process_list(
        &self,
        _: Request<GetProcessListRequest>,
    ) -> Result<Response<GetProcessListResponse>, Status> {
        Err(not_yet())
    }
    async fn run_command(
        &self,
        _: Request<RunCommandRequest>,
    ) -> Result<Response<RunCommandResponse>, Status> {
        Err(not_yet())
    }
    async fn get_command(
        &self,
        _: Request<GetCommandRequest>,
    ) -> Result<Response<CommandRecord>, Status> {
        Err(not_yet())
    }
    async fn list_commands(
        &self,
        _: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        Err(not_yet())
    }
}

type TokenRow = (Uuid, Uuid, DateTime<Utc>, DateTime<Utc>, bool);

impl OperatorApi {
    async fn pairing_token(&self, id: Uuid) -> Result<PairingToken, Status> {
        let row: TokenRow = sqlx::query_as(
            "SELECT id, environment_id, created_at, expires_at, revoked_at IS NOT NULL
             FROM pairing_tokens WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(internal)?;
        Ok(token_from_row(row))
    }
}

fn token_from_row((id, environment_id, created_at, expires_at, revoked): TokenRow) -> PairingToken {
    PairingToken {
        id: id.to_string(),
        environment_id: environment_id.to_string(),
        created_at: Some(ts(created_at)),
        expires_at: Some(ts(expires_at)),
        revoked,
    }
}

/// Keyset paging over UUIDv7 ids (which sort by creation): the token is the last id returned.
struct Page {
    after: Option<Uuid>,
    size: u32,
}

impl Page {
    fn new(page_size: u32, page_token: &str) -> Result<Self, Status> {
        let after = (!page_token.is_empty())
            .then(|| parse_id(page_token, "page_token"))
            .transpose()?;
        let size = match page_size {
            0 => DEFAULT_PAGE_SIZE,
            n => n.min(MAX_PAGE_SIZE),
        };
        Ok(Self { after, size })
    }

    /// Rows to fetch: one extra, to know whether another page exists.
    fn fetch(&self) -> i64 {
        i64::from(self.size) + 1
    }

    fn split<R>(&self, mut rows: Vec<R>, id: impl Fn(&R) -> Uuid) -> (Vec<R>, String) {
        if rows.len() > self.size as usize {
            rows.truncate(self.size as usize);
            let next = rows.last().map(|r| id(r).to_string()).unwrap_or_default();
            (rows, next)
        } else {
            (rows, String::new())
        }
    }
}

fn parse_id(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse()
        .map_err(|_| Status::invalid_argument(format!("{field} is not a valid id")))
}

fn ts(t: DateTime<Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: t.timestamp(),
        nanos: t.timestamp_subsec_nanos() as i32,
    }
}

fn header(value: &str) -> Result<MetadataValue<tonic::metadata::Ascii>, Status> {
    value.parse().map_err(internal)
}

fn not_yet() -> Status {
    Status::unimplemented("not implemented yet")
}

/// TLS for the Operator port, from a certificate (a public CA's, per the trust decision).
pub fn tls_config(cert_pem: &str, key_pem: &str) -> tonic::transport::ServerTlsConfig {
    tonic::transport::ServerTlsConfig::new()
        .identity(tonic::transport::Identity::from_pem(cert_pem, key_pem))
}

/// Serves the Operator port on `listener`: TLS (HTTP/2 via ALPN, which browsers use), grpc-web, and the
/// grpc-web mark (outside tonic-web, which rewrites the content type) that the cookie rule needs.
/// No CORS layer: only same-origin pages (the embedded web UI) can call it from a browser.
pub async fn serve(
    api: OperatorApi,
    tls: tonic::transport::ServerTlsConfig,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<()> {
    use rustyrmm_proto::operator::v1::operator_service_server::OperatorServiceServer;
    tonic::transport::Server::builder()
        .tls_config(tls)?
        .layer(tower::util::MapRequestLayer::new(|req| {
            auth::mark_grpc_web(req)
        }))
        .layer(tonic_web::GrpcWebLayer::new())
        .add_service(OperatorServiceServer::new(api))
        .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyrmm_proto::operator::v1::operator_service_client::OperatorServiceClient;
    use tonic::transport::{Certificate, Channel, ClientTlsConfig};

    const PASSWORD: &str = "correct horse battery";

    /// Runs the Operator port with a self-signed certificate standing in for a public CA's.
    async fn start(pool: PgPool) -> (OperatorServiceClient<Channel>, PgPool) {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let api = OperatorApi {
            pool: pool.clone(),
            agent_address: "rmm.example:7443".into(),
            ca_fingerprint: "ab".repeat(32),
        };
        tokio::spawn(serve(
            api,
            tls_config(&cert.pem(), &key.serialize_pem()),
            listener,
        ));
        operators::add_operator(&pool, "admin", PASSWORD)
            .await
            .unwrap();
        let channel = Channel::from_shared(format!("https://{addr}"))
            .unwrap()
            .tls_config(
                ClientTlsConfig::new()
                    .ca_certificate(Certificate::from_pem(cert.pem()))
                    .domain_name("localhost"),
            )
            .unwrap()
            .connect()
            .await
            .unwrap();
        (OperatorServiceClient::new(channel), pool)
    }

    fn authed<T>(msg: T, token: &str) -> Request<T> {
        let mut req = Request::new(msg);
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        req
    }

    async fn login(
        client: &mut OperatorServiceClient<Channel>,
        user: &str,
        pw: &str,
    ) -> Result<String, Status> {
        client
            .login(LoginRequest {
                username: user.into(),
                password: pw.into(),
            })
            .await
            .map(|r| r.into_inner().session_token)
    }

    #[sqlx::test]
    async fn login_then_set_up_environments_and_pairing_tokens(pool: PgPool) {
        let (mut client, pool) = start(pool).await;
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();

        for name in ["a", "b", "c"] {
            client
                .create_environment(authed(
                    CreateEnvironmentRequest { name: name.into() },
                    &token,
                ))
                .await
                .unwrap();
        }
        let dup = client
            .create_environment(authed(
                CreateEnvironmentRequest { name: "a".into() },
                &token,
            ))
            .await
            .unwrap_err();
        assert_eq!(dup.code(), tonic::Code::AlreadyExists);

        // Paging: 2 + 1, in creation order.
        let first = client
            .list_environments(authed(
                ListEnvironmentsRequest {
                    page_size: 2,
                    page_token: String::new(),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            first
                .environments
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(!first.next_page_token.is_empty());
        let second = client
            .list_environments(authed(
                ListEnvironmentsRequest {
                    page_size: 2,
                    page_token: first.next_page_token,
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(second.environments.len(), 1);
        assert_eq!(second.environments[0].name, "c");
        assert!(second.next_page_token.is_empty());

        let env = first.environments[0].id.clone();
        let created = client
            .create_pairing_token(authed(
                CreatePairingTokenRequest {
                    environment_id: env.clone(),
                    ttl: None,
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(created.agent_address, "rmm.example:7443");
        assert_eq!(created.ca_fingerprint_sha256.len(), 64);
        let env_id: Uuid = env.parse().unwrap();
        assert_eq!(
            pairing::redeem(&pool, &created.secret).await.unwrap(),
            Some(env_id)
        );

        let tokens = client
            .list_pairing_tokens(authed(
                ListPairingTokensRequest {
                    environment_id: env.clone(),
                    page_size: 0,
                    page_token: String::new(),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner()
            .pairing_tokens;
        assert_eq!(tokens.len(), 1);
        assert!(!tokens[0].revoked);

        let id = created.pairing_token.unwrap().id;
        client
            .revoke_pairing_token(authed(RevokePairingTokenRequest { id }, &token))
            .await
            .unwrap();
        assert_eq!(pairing::redeem(&pool, &created.secret).await.unwrap(), None);

        let missing = client
            .create_pairing_token(authed(
                CreatePairingTokenRequest {
                    environment_id: Uuid::now_v7().to_string(),
                    ttl: None,
                },
                &token,
            ))
            .await
            .unwrap_err();
        assert_eq!(missing.code(), tonic::Code::NotFound);
    }

    #[sqlx::test]
    async fn sessions_are_required_checked_and_end_at_logout(pool: PgPool) {
        let (mut client, pool) = start(pool).await;

        let none = client
            .list_environments(ListEnvironmentsRequest::default())
            .await
            .unwrap_err();
        assert_eq!(none.code(), tonic::Code::Unauthenticated);

        // Unknown user and wrong password fail the same way.
        let unknown = login(&mut client, "nobody", PASSWORD).await.unwrap_err();
        let wrong = login(&mut client, "admin", "not the password")
            .await
            .unwrap_err();
        assert_eq!(unknown.code(), tonic::Code::Unauthenticated);
        assert_eq!(unknown.message(), wrong.message());

        // A new Operator can log in.
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();
        client
            .add_operator(authed(
                AddOperatorRequest {
                    username: "bob".into(),
                    password: PASSWORD.into(),
                },
                &token,
            ))
            .await
            .unwrap();
        let bob = login(&mut client, "bob", PASSWORD).await.unwrap();

        // Logout ends that session only.
        client.logout(authed(LogoutRequest {}, &bob)).await.unwrap();
        let after = client
            .list_environments(authed(ListEnvironmentsRequest::default(), &bob))
            .await
            .unwrap_err();
        assert_eq!(after.code(), tonic::Code::Unauthenticated);
        client
            .list_environments(authed(ListEnvironmentsRequest::default(), &token))
            .await
            .unwrap();

        // Expired sessions stop working.
        sqlx::query("UPDATE operator_sessions SET expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        let expired = client
            .list_environments(authed(ListEnvironmentsRequest::default(), &token))
            .await
            .unwrap_err();
        assert_eq!(expired.code(), tonic::Code::Unauthenticated);
    }

    #[sqlx::test]
    async fn browser_cookie_is_set_on_grpc_web_login_and_only_counts_on_grpc_web(pool: PgPool) {
        operators::add_operator(&pool, "admin", PASSWORD)
            .await
            .unwrap();
        let api = OperatorApi {
            pool: pool.clone(),
            agent_address: String::new(),
            ca_fingerprint: String::new(),
        };
        let marked = |web: bool, cookie: Option<&str>| {
            let mut req = Request::new(ListEnvironmentsRequest::default());
            req.extensions_mut().insert(auth::GrpcWeb(web));
            if let Some(c) = cookie {
                req.metadata_mut().insert("cookie", c.parse().unwrap());
            }
            req
        };

        let mut login = Request::new(LoginRequest {
            username: "admin".into(),
            password: PASSWORD.into(),
        });
        login.extensions_mut().insert(auth::GrpcWeb(true));
        let resp = api.login(login).await.unwrap();
        let set_cookie = resp
            .metadata()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            set_cookie.contains("HttpOnly")
                && set_cookie.contains("SameSite=Strict")
                && set_cookie.contains("Secure")
        );
        let cookie = set_cookie.split(';').next().unwrap().to_owned();

        assert!(api
            .list_environments(marked(true, Some(&cookie)))
            .await
            .is_ok());
        let plain = api
            .list_environments(marked(false, Some(&cookie)))
            .await
            .unwrap_err();
        assert_eq!(
            plain.code(),
            tonic::Code::Unauthenticated,
            "cookie ignored off grpc-web"
        );

        // The outer layer marks requests by their original content type.
        let req = |ct: &str| {
            tonic::codegen::http::Request::builder()
                .header("content-type", ct)
                .body(())
                .unwrap()
        };
        let mark = |r| {
            auth::mark_grpc_web(r)
                .extensions()
                .get::<auth::GrpcWeb>()
                .unwrap()
                .0
        };
        assert!(mark(req("application/grpc-web+proto")));
        assert!(mark(req("application/grpc-web-text")));
        assert!(!mark(req("application/grpc")));
        assert!(!mark(req("text/plain")));
    }
}
