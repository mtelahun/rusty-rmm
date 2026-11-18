//! The Operator port's gRPC service (used by the CLI directly and the web UI via grpc-web).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use prost::Message;
use rustyrmm_proto::common::v1::{
    CommandResult, DiskSample, InterfaceSample, Interpreter, Inventory, Sample,
};
use rustyrmm_proto::operator::v1::operator_service_server::OperatorService;
use rustyrmm_proto::operator::v1::*;
use sqlx::PgPool;
use tonic::metadata::MetadataValue;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::auth::{self, authenticate, internal};
use crate::commands::{self, DispatchError, NewCommand};
use crate::operators::{self, AddOperatorError};
use crate::pairing;
use crate::sessions::{LiveError, Sessions};

pub const DEFAULT_PAIRING_TOKEN_TTL: chrono::Duration = chrono::Duration::days(7);
const DEFAULT_PAGE_SIZE: u32 = 100;
const MAX_PAGE_SIZE: u32 = 1000;
/// Limits from Grilling: Command execution limits.
const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const DEFAULT_COMMAND_TIMEOUT_SECS: u32 = 300;
const MAX_COMMAND_TIMEOUT_SECS: u32 = 3600;
const PROCESS_LIST_WAIT: Duration = Duration::from_secs(10);
/// ListSamples ranges up to this long return raw Samples; longer ones return rollups.
const RAW_SAMPLES_MAX_RANGE: chrono::Duration = chrono::Duration::hours(24);

pub struct OperatorApi {
    pub pool: PgPool,
    pub sessions: Arc<Sessions>,
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

    async fn list_endpoints(
        &self,
        req: Request<ListEndpointsRequest>,
    ) -> Result<Response<ListEndpointsResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let environment_id = req
            .environment_id
            .as_deref()
            .map(|id| parse_id(id, "environment_id"))
            .transpose()?;
        let page = Page::new(req.page_size, &req.page_token)?;
        let rows: Vec<EndpointRow> = sqlx::query_as(&format!(
            "{ENDPOINT_SELECT}
             WHERE ($1::uuid IS NULL OR e.environment_id = $1) AND ($2::uuid IS NULL OR e.id > $2)
             ORDER BY e.id LIMIT $3"
        ))
        .bind(environment_id)
        .bind(page.after)
        .bind(page.fetch())
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let (rows, next_page_token) = page.split(rows, |r| r.0);
        Ok(Response::new(ListEndpointsResponse {
            endpoints: rows.into_iter().map(|r| self.endpoint(r)).collect(),
            next_page_token,
        }))
    }

    async fn get_endpoint(
        &self,
        req: Request<GetEndpointRequest>,
    ) -> Result<Response<Endpoint>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().id, "id")?;
        let row: EndpointRow = sqlx::query_as(&format!("{ENDPOINT_SELECT} WHERE e.id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)?
            .ok_or_else(no_endpoint)?;
        Ok(Response::new(self.endpoint(row)))
    }

    async fn revoke_endpoint(
        &self,
        req: Request<RevokeEndpointRequest>,
    ) -> Result<Response<RevokeEndpointResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().id, "id")?;
        let found = sqlx::query(
            "UPDATE endpoints SET revoked_at = coalesce(revoked_at, now()) WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(internal)?
        .rows_affected();
        if found == 0 {
            return Err(no_endpoint());
        }
        self.sessions.disconnect(id); // Marked Revoked first, so it can't reconnect.
        Ok(Response::new(RevokeEndpointResponse {}))
    }

    async fn delete_endpoint(
        &self,
        req: Request<DeleteEndpointRequest>,
    ) -> Result<Response<DeleteEndpointResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().id, "id")?;
        // Samples, Inventory and certificates cascade; Commands are kept (no foreign key).
        let found = sqlx::query("DELETE FROM endpoints WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(internal)?
            .rows_affected();
        if found == 0 {
            return Err(no_endpoint());
        }
        self.sessions.disconnect(id); // Its certificate is no longer on record.
        Ok(Response::new(DeleteEndpointResponse {}))
    }

    async fn list_samples(
        &self,
        req: Request<ListSamplesRequest>,
    ) -> Result<Response<ListSamplesResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let endpoint_id = parse_id(&req.endpoint_id, "endpoint_id")?;
        let (Some(start), Some(end)) = (req.start.and_then(from_ts), req.end.and_then(from_ts))
        else {
            return Err(Status::invalid_argument("start and end are required"));
        };
        if start >= end {
            return Err(Status::invalid_argument("start must be before end"));
        }
        let size = Page::new(req.page_size, "")?.size;
        // Page tokens are the last row's time; resume strictly after it.
        let after = match req.page_token.as_str() {
            "" => start - chrono::Duration::microseconds(1),
            t => t
                .parse::<DateTime<Utc>>()
                .map_err(|_| Status::invalid_argument("page_token is not valid"))?,
        };
        // Short ranges get raw 30-second Samples; longer ones the 5-minute rollups (averages).
        let raw = end - start <= RAW_SAMPLES_MAX_RANGE;
        let sql = if raw {
            "SELECT collected_at, cpu_usage_percent, memory_used_bytes, memory_total_bytes, disks, interfaces
             FROM samples WHERE endpoint_id = $1 AND collected_at > $2 AND collected_at < $3
             ORDER BY collected_at LIMIT $4"
        } else {
            "SELECT r.bucket_start, r.cpu_usage_percent_avg, r.memory_used_bytes_avg,
                    coalesce((SELECT memory_total_bytes FROM samples s WHERE s.endpoint_id = r.endpoint_id
                              ORDER BY collected_at DESC LIMIT 1), 0),
                    r.disks, r.interfaces
             FROM sample_rollups r WHERE r.endpoint_id = $1 AND r.bucket_start > $2 AND r.bucket_start < $3
             ORDER BY r.bucket_start LIMIT $4"
        };
        let rows: Vec<SampleRow> = sqlx::query_as(sql)
            .bind(endpoint_id)
            .bind(after)
            .bind(end)
            .bind(i64::from(size) + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        let more = rows.len() > size as usize;
        let rows = &rows[..rows.len().min(size as usize)];
        let next_page_token = match (more, rows.last()) {
            (true, Some(last)) => last.0.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            _ => String::new(),
        };
        Ok(Response::new(ListSamplesResponse {
            samples: rows.iter().map(|r| sample_from_row(r, raw)).collect(),
            next_page_token,
        }))
    }

    async fn get_process_list(
        &self,
        req: Request<GetProcessListRequest>,
    ) -> Result<Response<GetProcessListResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().endpoint_id, "endpoint_id")?;
        match self
            .sessions
            .request_process_list(id, PROCESS_LIST_WAIT)
            .await
        {
            Ok(processes) => Ok(Response::new(GetProcessListResponse {
                processes,
                received_at: Some(ts(Utc::now())),
            })),
            Err(e) => Err(self.live_error(id, e).await),
        }
    }

    async fn run_command(
        &self,
        req: Request<RunCommandRequest>,
    ) -> Result<Response<RunCommandResponse>, Status> {
        let operator = authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let endpoint_id = parse_id(&req.endpoint_id, "endpoint_id")?;
        let interpreter = Interpreter::try_from(req.interpreter)
            .ok()
            .filter(|i| *i != Interpreter::Unspecified)
            .ok_or_else(|| Status::invalid_argument("interpreter is required"))?;
        if req.script.len() > MAX_SCRIPT_BYTES {
            return Err(Status::invalid_argument("script is larger than 64 KiB"));
        }
        let timeout_seconds = match req.timeout_seconds {
            0 => DEFAULT_COMMAND_TIMEOUT_SECS,
            t if t > MAX_COMMAND_TIMEOUT_SECS => {
                return Err(Status::invalid_argument("timeout_seconds is above 3600"))
            }
            t => t,
        };
        let cmd = NewCommand {
            endpoint_id,
            operator_username: operator.username,
            interpreter,
            script: req.script,
            timeout_seconds,
        };
        match commands::dispatch(&self.pool, &self.sessions, cmd).await {
            Ok(id) => Ok(Response::new(RunCommandResponse {
                command_id: id.to_string(),
            })),
            Err(DispatchError::Live(e)) => Err(self.live_error(endpoint_id, e).await),
            Err(DispatchError::Db(e)) => Err(internal(e)),
        }
    }

    async fn get_command(
        &self,
        req: Request<GetCommandRequest>,
    ) -> Result<Response<CommandRecord>, Status> {
        authenticate(&self.pool, &req).await?;
        let id = parse_id(&req.into_inner().id, "id")?;
        let row: CommandRow = sqlx::query_as(&format!("{COMMAND_SELECT} WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)?
            .ok_or_else(|| Status::not_found("no such Command"))?;
        Ok(Response::new(command_from_row(row)))
    }

    async fn list_commands(
        &self,
        req: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        authenticate(&self.pool, &req).await?;
        let req = req.into_inner();
        let endpoint_id = req
            .endpoint_id
            .as_deref()
            .map(|id| parse_id(id, "endpoint_id"))
            .transpose()?;
        let page = Page::new(req.page_size, &req.page_token)?;
        // Newest first: page backwards through the time-ordered ids.
        let rows: Vec<CommandRow> = sqlx::query_as(&format!(
            "{COMMAND_SELECT}
             WHERE ($1::uuid IS NULL OR endpoint_id = $1) AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3"
        ))
        .bind(endpoint_id)
        .bind(page.after)
        .bind(page.fetch())
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let (rows, next_page_token) = page.split(rows, |r| r.id);
        Ok(Response::new(ListCommandsResponse {
            commands: rows.into_iter().map(command_from_row).collect(),
            next_page_token,
        }))
    }
}

/// Endpoint columns plus its latest Inventory (encoded), if any.
const ENDPOINT_SELECT: &str =
    "SELECT e.id, e.environment_id, e.hostname, e.paired_at, e.last_seen_at,
        e.revoked_at IS NOT NULL,
        (SELECT i.inventory FROM inventories i WHERE i.endpoint_id = e.id
         ORDER BY i.received_at DESC, i.id DESC LIMIT 1)
     FROM endpoints e";
type EndpointRow = (
    Uuid,
    Uuid,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    bool,
    Option<Vec<u8>>,
);

const COMMAND_SELECT: &str = "SELECT id, endpoint_id, endpoint_hostname, operator_username,
        interpreter, script, timeout_seconds, status, created_at, exit_code, stdout, stderr,
        truncated, timed_out, started_at, finished_at
     FROM commands";

#[derive(sqlx::FromRow)]
struct CommandRow {
    id: Uuid,
    endpoint_id: Uuid,
    endpoint_hostname: String,
    operator_username: String,
    interpreter: i16,
    script: String,
    timeout_seconds: i32,
    status: i16,
    created_at: DateTime<Utc>,
    exit_code: Option<i32>,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    truncated: Option<bool>,
    timed_out: Option<bool>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

fn command_from_row(r: CommandRow) -> CommandRecord {
    let completed = r.status == CommandStatus::Completed as i16;
    CommandRecord {
        id: r.id.to_string(),
        endpoint_id: r.endpoint_id.to_string(),
        endpoint_hostname: r.endpoint_hostname,
        operator_username: r.operator_username,
        interpreter: r.interpreter.into(),
        script: r.script,
        timeout_seconds: r.timeout_seconds.try_into().unwrap_or(0),
        status: r.status.into(),
        created_at: Some(ts(r.created_at)),
        result: completed.then(|| CommandResult {
            id: r.id.to_string(),
            exit_code: r.exit_code,
            stdout: r.stdout.unwrap_or_default(),
            stderr: r.stderr.unwrap_or_default(),
            truncated: r.truncated.unwrap_or(false),
            timed_out: r.timed_out.unwrap_or(false),
            started_at: r.started_at.map(ts),
            finished_at: r.finished_at.map(ts),
        }),
    }
}

/// (time, cpu, memory used, memory total, disks, interfaces) from samples or sample_rollups.
type SampleRow = (
    DateTime<Utc>,
    f32,
    i64,
    i64,
    serde_json::Value,
    serde_json::Value,
);

/// Rollup JSON names averaged fields `*_avg`; raw Samples use the plain names.
fn sample_from_row(r: &SampleRow, raw: bool) -> Sample {
    let field = |v: &serde_json::Value, name: &str| {
        let key = if raw {
            name.to_owned()
        } else {
            format!("{name}_avg")
        };
        v.get(&key).and_then(serde_json::Value::as_u64).unwrap_or(0)
    };
    let items = |v: &serde_json::Value| v.as_array().cloned().unwrap_or_default();
    let name = |v: &serde_json::Value| {
        v.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_owned()
    };
    Sample {
        collected_at: Some(ts(r.0)),
        cpu_usage_percent: r.1,
        memory_used_bytes: r.2.try_into().unwrap_or(0),
        memory_total_bytes: r.3.try_into().unwrap_or(0),
        disks: items(&r.4)
            .iter()
            .map(|d| DiskSample {
                name: name(d),
                // Rollups keep the maximum used space, not an average.
                used_bytes: d
                    .get(if raw { "used_bytes" } else { "used_bytes_max" })
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                read_bytes_per_sec: field(d, "read_bytes_per_sec"),
                write_bytes_per_sec: field(d, "write_bytes_per_sec"),
            })
            .collect(),
        interfaces: items(&r.5)
            .iter()
            .map(|i| InterfaceSample {
                name: name(i),
                rx_bytes_per_sec: field(i, "rx_bytes_per_sec"),
                tx_bytes_per_sec: field(i, "tx_bytes_per_sec"),
                link_speed_bps: i.get("link_speed_bps").and_then(serde_json::Value::as_u64),
            })
            .collect(),
    }
}

fn no_endpoint() -> Status {
    Status::not_found("no such Endpoint")
}

fn from_ts(t: prost_types::Timestamp) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(t.seconds, u32::try_from(t.nanos).ok()?)
}

type TokenRow = (Uuid, Uuid, DateTime<Utc>, DateTime<Utc>, bool);

impl OperatorApi {
    fn endpoint(
        &self,
        (id, environment_id, hostname, paired_at, last_seen_at, revoked, inventory): EndpointRow,
    ) -> Endpoint {
        Endpoint {
            id: id.to_string(),
            environment_id: environment_id.to_string(),
            hostname,
            online: self.sessions.is_online(id),
            revoked,
            paired_at: Some(ts(paired_at)),
            last_seen_at: last_seen_at.map(ts),
            inventory: inventory.and_then(|b| Inventory::decode(b.as_slice()).ok()),
        }
    }

    /// Maps a live-Session failure to a status, telling an unknown Endpoint from an Offline one.
    async fn live_error(&self, endpoint_id: Uuid, e: LiveError) -> Status {
        if e == LiveError::Timeout {
            return Status::deadline_exceeded(e.to_string());
        }
        let exists: Result<bool, _> =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM endpoints WHERE id = $1)")
                .bind(endpoint_id)
                .fetch_one(&self.pool)
                .await;
        match exists {
            Ok(true) => Status::unavailable(e.to_string()),
            Ok(false) => no_endpoint(),
            Err(err) => internal(err),
        }
    }

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
        let (client, pool, _) = start_with_sessions(pool).await;
        (client, pool)
    }

    async fn start_with_sessions(
        pool: PgPool,
    ) -> (OperatorServiceClient<Channel>, PgPool, Arc<Sessions>) {
        let sessions = Arc::new(Sessions::default());
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let api = OperatorApi {
            pool: pool.clone(),
            sessions: sessions.clone(),
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
        (OperatorServiceClient::new(channel), pool, sessions)
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
            sessions: Default::default(),
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

    // --- Endpoint, Monitoring and Command calls ---

    use rustyrmm_proto::agent::v1::{server_message, ProcessList};
    use rustyrmm_proto::common::v1::ProcessInfo;

    async fn env_and_endpoints(pool: &PgPool, hostnames: &[&str]) -> Vec<Uuid> {
        let env = Uuid::now_v7();
        sqlx::query("INSERT INTO environments (id, name) VALUES ($1, 'lab')")
            .bind(env)
            .execute(pool)
            .await
            .unwrap();
        let mut ids = Vec::new();
        for h in hostnames {
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO endpoints (id, environment_id, hostname) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(env)
                .bind(*h)
                .execute(pool)
                .await
                .unwrap();
            ids.push(id);
        }
        ids
    }

    /// Registers a fake live Session; returns what the Server sends to the "Agent".
    fn online(
        sessions: &Sessions,
        ep: Uuid,
    ) -> (
        tokio::sync::mpsc::Receiver<Result<rustyrmm_proto::agent::v1::ServerMessage, Status>>,
        crate::sessions::Registration,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let reg = sessions.register(ep, tx);
        (rx, reg)
    }

    #[sqlx::test]
    async fn endpoints_list_get_revoke_and_delete(pool: PgPool) {
        let (mut client, pool, sessions) = start_with_sessions(pool).await;
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();
        let eps = env_and_endpoints(&pool, &["a", "b", "c"]).await;
        let inv = Inventory {
            hostname: "a".into(),
            timezone: "UTC".into(),
            ..Default::default()
        };
        sqlx::query("INSERT INTO inventories (endpoint_id, collected_at, received_at, inventory) VALUES ($1, now(), now(), $2)")
            .bind(eps[0])
            .bind(inv.encode_to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let (_rx, reg) = online(&sessions, eps[0]);

        let page = client
            .list_endpoints(authed(
                ListEndpointsRequest {
                    environment_id: None,
                    page_size: 2,
                    page_token: String::new(),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            page.endpoints
                .iter()
                .map(|e| e.hostname.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(page.endpoints[0].online && !page.endpoints[1].online);
        assert_eq!(
            page.endpoints[0].inventory.as_ref().unwrap().timezone,
            "UTC"
        );
        assert!(page.endpoints[1].inventory.is_none());
        assert!(!page.next_page_token.is_empty());

        let get = |id: Uuid| authed(GetEndpointRequest { id: id.to_string() }, &token);
        assert_eq!(
            client
                .get_endpoint(get(eps[2]))
                .await
                .unwrap()
                .into_inner()
                .hostname,
            "c"
        );

        // Revoke: marked, and the live Session is closed.
        client
            .revoke_endpoint(authed(
                RevokeEndpointRequest {
                    id: eps[0].to_string(),
                },
                &token,
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), reg.close.notified())
            .await
            .unwrap();
        let revoked = client.get_endpoint(get(eps[0])).await.unwrap().into_inner();
        assert!(revoked.revoked && !revoked.online);

        // Delete: gone, but its Commands stay on record.
        sqlx::query("INSERT INTO commands (id, endpoint_id, endpoint_hostname, operator_username, interpreter, script, timeout_seconds, status) VALUES ($1, $2, 'b', 'admin', 1, 'true', 5, 2)")
            .bind(Uuid::now_v7())
            .bind(eps[1])
            .execute(&pool)
            .await
            .unwrap();
        client
            .delete_endpoint(authed(
                DeleteEndpointRequest {
                    id: eps[1].to_string(),
                },
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(
            client.get_endpoint(get(eps[1])).await.unwrap_err().code(),
            tonic::Code::NotFound
        );
        let kept = client
            .list_commands(authed(
                ListCommandsRequest {
                    endpoint_id: Some(eps[1].to_string()),
                    page_size: 0,
                    page_token: String::new(),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner()
            .commands;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].endpoint_hostname, "b");
    }

    #[sqlx::test]
    async fn samples_come_raw_for_short_ranges_and_rolled_up_for_long_ones(pool: PgPool) {
        let (mut client, pool, _) = start_with_sessions(pool).await;
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();
        let ep = env_and_endpoints(&pool, &["a"]).await[0];
        for mins in [10, 20, 30] {
            sqlx::query("INSERT INTO samples (endpoint_id, collected_at, received_at, cpu_usage_percent, memory_used_bytes, memory_total_bytes, disks, interfaces) VALUES ($1, now() - make_interval(mins => $2), now(), 10, 100, 1000, '[{\"name\":\"/\",\"used_bytes\":7,\"read_bytes_per_sec\":5,\"write_bytes_per_sec\":6}]', '[{\"name\":\"eth0\",\"rx_bytes_per_sec\":1,\"tx_bytes_per_sec\":2,\"link_speed_bps\":null}]')")
                .bind(ep)
                .bind(mins)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO sample_rollups (endpoint_id, bucket_start, cpu_usage_percent_avg, cpu_usage_percent_max, memory_used_bytes_avg, memory_used_bytes_max, disks, interfaces) VALUES ($1, now() - interval '3 days', 40, 90, 500, 600, '[{\"name\":\"/\",\"used_bytes_max\":9,\"read_bytes_per_sec_avg\":11,\"read_bytes_per_sec_max\":20,\"write_bytes_per_sec_avg\":12,\"write_bytes_per_sec_max\":30}]', '[]')")
            .bind(ep)
            .execute(&pool)
            .await
            .unwrap();
        let now = Utc::now();
        let range = |hours: i64, page_size: u32, page_token: String| {
            authed(
                ListSamplesRequest {
                    endpoint_id: ep.to_string(),
                    start: Some(ts(now - chrono::Duration::hours(hours))),
                    end: Some(ts(now)),
                    page_size,
                    page_token,
                },
                &token,
            )
        };

        let p1 = client
            .list_samples(range(1, 2, String::new()))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(p1.samples.len(), 2);
        assert_eq!(p1.samples[0].disks[0].read_bytes_per_sec, 5);
        assert_eq!(p1.samples[0].interfaces[0].link_speed_bps, None);
        let p2 = client
            .list_samples(range(1, 2, p1.next_page_token))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(p2.samples.len(), 1);
        assert!(p2.next_page_token.is_empty());

        let week = client
            .list_samples(range(24 * 7, 0, String::new()))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(week.samples.len(), 1, "a 7-day range reads the rollups");
        let s = &week.samples[0];
        assert_eq!(
            (
                s.cpu_usage_percent,
                s.memory_used_bytes,
                s.memory_total_bytes
            ),
            (40.0, 500, 1000)
        );
        assert_eq!(
            (s.disks[0].used_bytes, s.disks[0].read_bytes_per_sec),
            (9, 11)
        );

        let backwards = client
            .list_samples(range(-1, 0, String::new()))
            .await
            .unwrap_err();
        assert_eq!(backwards.code(), tonic::Code::InvalidArgument);
    }

    #[sqlx::test]
    async fn run_command_validates_dispatches_and_records(pool: PgPool) {
        let (mut client, pool, sessions) = start_with_sessions(pool).await;
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();
        let ep = env_and_endpoints(&pool, &["a"]).await[0];
        let run = |script: String, timeout_seconds: u32, interpreter: Interpreter| {
            authed(
                RunCommandRequest {
                    endpoint_id: ep.to_string(),
                    interpreter: interpreter as i32,
                    script,
                    timeout_seconds,
                },
                &token,
            )
        };
        let code = |r: Result<Response<RunCommandResponse>, Status>| r.unwrap_err().code();
        assert_eq!(
            code(
                client
                    .run_command(run("x".repeat(64 * 1024 + 1), 0, Interpreter::Sh))
                    .await
            ),
            tonic::Code::InvalidArgument
        );
        assert_eq!(
            code(
                client
                    .run_command(run("true".into(), 3601, Interpreter::Sh))
                    .await
            ),
            tonic::Code::InvalidArgument
        );
        assert_eq!(
            code(
                client
                    .run_command(run("true".into(), 0, Interpreter::Unspecified))
                    .await
            ),
            tonic::Code::InvalidArgument
        );
        assert_eq!(
            code(
                client
                    .run_command(run("true".into(), 0, Interpreter::Sh))
                    .await
            ),
            tonic::Code::Unavailable,
            "Offline"
        );

        let (mut rx, _reg) = online(&sessions, ep);
        let id = client
            .run_command(run("echo hi".into(), 0, Interpreter::Sh))
            .await
            .unwrap()
            .into_inner()
            .command_id;
        let Some(server_message::Message::Command(cmd)) = rx.recv().await.unwrap().unwrap().message
        else {
            panic!("expected a Command");
        };
        assert_eq!(cmd.timeout_seconds, 300, "0 becomes the 5-minute default");
        let pending = client
            .get_command(authed(GetCommandRequest { id: id.clone() }, &token))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(pending.status, CommandStatus::Dispatched as i32);
        assert!(pending.result.is_none());

        // The "Agent" reports back.
        commands::record_result(
            &pool,
            ep,
            &CommandResult {
                id: id.clone(),
                exit_code: Some(0),
                stdout: b"hi\n".to_vec(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let done = client
            .get_command(authed(GetCommandRequest { id }, &token))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(done.status, CommandStatus::Completed as i32);
        assert_eq!(done.operator_username, "admin");
        let result = done.result.unwrap();
        assert_eq!(
            (result.exit_code, result.stdout.as_slice()),
            (Some(0), b"hi\n".as_slice())
        );
    }

    #[sqlx::test]
    async fn process_list_is_live_and_errors_say_why(pool: PgPool) {
        let (mut client, pool, sessions) = start_with_sessions(pool).await;
        let token = login(&mut client, "admin", PASSWORD).await.unwrap();
        let ep = env_and_endpoints(&pool, &["a"]).await[0];
        let ask = |id: Uuid| {
            authed(
                GetProcessListRequest {
                    endpoint_id: id.to_string(),
                },
                &token,
            )
        };

        assert_eq!(
            client.get_process_list(ask(ep)).await.unwrap_err().code(),
            tonic::Code::Unavailable
        );
        assert_eq!(
            client
                .get_process_list(ask(Uuid::now_v7()))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::NotFound
        );

        let (mut rx, _reg) = online(&sessions, ep);
        let agent = {
            let sessions = sessions.clone();
            tokio::spawn(async move {
                let Some(server_message::Message::ProcessListRequest(req)) =
                    rx.recv().await.unwrap().unwrap().message
                else {
                    panic!("expected a ProcessListRequest");
                };
                sessions.complete_process_list(
                    ep,
                    ProcessList {
                        request_id: req.request_id,
                        processes: vec![ProcessInfo {
                            pid: 1,
                            name: "init".into(),
                            ..Default::default()
                        }],
                    },
                );
            })
        };
        let list = client.get_process_list(ask(ep)).await.unwrap().into_inner();
        agent.await.unwrap();
        assert_eq!(list.processes[0].name, "init");
        assert!(list.received_at.is_some());
    }
}
