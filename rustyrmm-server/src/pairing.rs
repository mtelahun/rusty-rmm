//! Pairing Tokens: reusable, expiring secrets tied to one Environment. Stored only as hashes.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// Creates a Pairing Token and returns (id, secret). The secret is shown once and never stored.
pub async fn create_token(
    pool: &PgPool,
    environment_id: Uuid,
    ttl: chrono::Duration,
) -> sqlx::Result<(Uuid, String)> {
    let secret = hex::encode(rand::random::<[u8; 32]>());
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO pairing_tokens (id, environment_id, secret_hash, expires_at)
         VALUES ($1, $2, $3, now() + $4)",
    )
    .bind(id)
    .bind(environment_id)
    .bind(hash_secret(&secret))
    .bind(ttl)
    .execute(pool)
    .await?;
    Ok((id, secret))
}

/// Returns the Environment of a valid (unrevoked, unexpired) Pairing Token.
pub async fn redeem(pool: &PgPool, secret: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "SELECT environment_id FROM pairing_tokens
         WHERE secret_hash = $1 AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(hash_secret(secret))
    .fetch_optional(pool)
    .await
}
