//! Operator sessions: login, logout, and authenticating Operator-port calls.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::PasswordVerifier;
use argon2::Argon2;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tonic::{Request, Status};
use uuid::Uuid;

use crate::pairing::hash_secret;

pub const SESSION_TTL: chrono::Duration = chrono::Duration::hours(12);
pub const COOKIE: &str = "rustyrmm_session";

/// Request extension set by the Operator port's outer layer: whether the browser sent the call
/// as grpc-web. tonic-web rewrites the content type before the service sees it, and clients
/// can't set extensions, so this can't be forged.
#[derive(Clone, Copy, Debug)]
pub struct GrpcWeb(pub bool);

/// Marks each request with whether it arrived as grpc-web. Wrap it outside tonic-web.
pub fn mark_grpc_web<B>(
    mut req: tonic::codegen::http::Request<B>,
) -> tonic::codegen::http::Request<B> {
    let web = req
        .headers()
        .get(tonic::codegen::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/grpc-web"));
    req.extensions_mut().insert(GrpcWeb(web));
    req
}

pub struct Operator {
    pub id: Uuid,
    pub username: String,
    pub is_admin: bool,
    pub email: Option<String>,
    pub token_hash: Vec<u8>,
}

/// Checks a password and starts a session. Wrong username and wrong password are
/// indistinguishable, including in timing.
pub async fn login(
    pool: &PgPool,
    username: &str,
    password: &str,
) -> Result<(String, DateTime<Utc>), Status> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM operators WHERE username = $1")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
    // Verify against a dummy hash for unknown users, so the work done is the same.
    let (id, hash) = row.unwrap_or_else(|| (Uuid::nil(), DUMMY_HASH.clone()));
    let verified = PasswordHash::new(&hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false);
    if !verified || id.is_nil() {
        return Err(Status::unauthenticated("wrong username or password"));
    }

    let token = hex::encode(rand::random::<[u8; 32]>());
    let expires_at = Utc::now() + SESSION_TTL;
    sqlx::query(
        "INSERT INTO operator_sessions (token_hash, operator_id, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(hash_secret(&token))
    .bind(id)
    .bind(expires_at)
    .execute(pool)
    .await
    .map_err(internal)?;
    Ok((token, expires_at))
}

pub async fn logout(pool: &PgPool, operator: &Operator) -> Result<(), Status> {
    sqlx::query("UPDATE operator_sessions SET revoked_at = now() WHERE token_hash = $1")
        .bind(&operator.token_hash)
        .execute(pool)
        .await
        .map_err(internal)?;
    Ok(())
}

/// Identifies the calling Operator from `authorization: Bearer <token>` (CLI), or from the
/// session cookie, which is accepted only on grpc-web calls (the CSRF rule: a cross-site page
/// can't send a grpc-web content type without a CORS preflight, and no origin is allowed).
pub async fn authenticate<T>(pool: &PgPool, req: &Request<T>) -> Result<Operator, Status> {
    let token = bearer(req)
        .or_else(|| {
            let web = req.extensions().get::<GrpcWeb>().is_some_and(|w| w.0);
            web.then(|| session_cookie(req)).flatten()
        })
        .ok_or_else(|| Status::unauthenticated("log in first"))?;
    let token_hash = hash_secret(&token);
    let row: Option<(Uuid, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT o.id, o.username, o.is_admin, o.email FROM operator_sessions s JOIN operators o ON o.id = s.operator_id
         WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(internal)?;
    let (id, username, is_admin, email) =
        row.ok_or_else(|| Status::unauthenticated("session expired or logged out"))?;
    Ok(Operator {
        id,
        username,
        is_admin,
        email,
        token_hash,
    })
}

fn bearer<T>(req: &Request<T>) -> Option<String> {
    let value = req.metadata().get("authorization")?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

fn session_cookie<T>(req: &Request<T>) -> Option<String> {
    let cookies = req.metadata().get("cookie")?.to_str().ok()?;
    cookies
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
}

/// `Set-Cookie` value for a browser session; `max_age` 0 clears it.
pub fn set_cookie(token: &str, max_age: i64) -> String {
    format!("{COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}")
}

pub fn is_grpc_web<T>(req: &Request<T>) -> bool {
    req.extensions().get::<GrpcWeb>().is_some_and(|w| w.0)
}

/// argon2id hash of a random secret nobody knows, made with the same parameters as real
/// password hashes; used to equalize login timing.
static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    use argon2::password_hash::PasswordHasher;
    Argon2::default()
        .hash_password(&rand::random::<[u8; 32]>())
        .expect("hashing random bytes")
        .to_string()
});

pub fn internal(e: impl std::fmt::Display) -> Status {
    // shortcut: stderr until the Server adopts structured logging.
    eprintln!("internal error: {e}");
    Status::internal("internal error")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(headers: &[(&'static str, &str)], web: bool) -> Request<()> {
        let mut req = Request::new(());
        for (k, v) in headers {
            req.metadata_mut().insert(*k, v.parse().unwrap());
        }
        req.extensions_mut().insert(GrpcWeb(web));
        req
    }

    #[test]
    fn cookie_counts_only_on_grpc_web_calls() {
        let cookie = [("cookie", "theme=dark; rustyrmm_session=abc; x=y")];
        assert_eq!(bearer(&with(&cookie, true)), None);
        assert_eq!(session_cookie(&with(&cookie, true)).as_deref(), Some("abc"));
        assert!(is_grpc_web(&with(&cookie, true)));
        assert!(!is_grpc_web(&with(&cookie, false)));
        assert!(
            !is_grpc_web(&Request::new(())),
            "unmarked requests are not grpc-web"
        );
    }

    #[test]
    fn bearer_needs_the_scheme() {
        assert_eq!(
            bearer(&with(&[("authorization", "Bearer t0k")], false)).as_deref(),
            Some("t0k")
        );
        assert_eq!(bearer(&with(&[("authorization", "t0k")], false)), None);
    }

    #[test]
    fn dummy_hash_parses() {
        assert!(PasswordHash::new(&DUMMY_HASH).is_ok());
    }
}
