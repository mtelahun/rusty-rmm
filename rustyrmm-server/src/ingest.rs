//! Stores what Agents send over their Sessions.

use chrono::{DateTime, Utc};
use prost::Message;
use rustyrmm_proto::common::v1::{Inventory, Sample};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Stores the Inventory only if it differs from the latest one (ignoring `collected_at`),
/// and keeps the Endpoint's hostname current. Returns whether a row was written.
pub async fn inventory(pool: &PgPool, endpoint_id: Uuid, inv: &Inventory) -> sqlx::Result<bool> {
    let latest: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT inventory FROM inventories WHERE endpoint_id = $1
         ORDER BY received_at DESC, id DESC LIMIT 1",
    )
    .bind(endpoint_id)
    .fetch_optional(pool)
    .await?;
    let unchanged = latest
        .and_then(|bytes| Inventory::decode(bytes.as_slice()).ok())
        .is_some_and(|prev| without_timestamp(prev) == without_timestamp(inv.clone()));
    if unchanged {
        return Ok(false);
    }

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO inventories (endpoint_id, collected_at, received_at, inventory)
         VALUES ($1, $2, now(), $3)",
    )
    .bind(endpoint_id)
    .bind(collected_at(inv.collected_at.as_ref()))
    .bind(inv.encode_to_vec())
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE endpoints SET hostname = $2 WHERE id = $1")
        .bind(endpoint_id)
        .bind(&inv.hostname)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn sample(pool: &PgPool, endpoint_id: Uuid, s: &Sample) -> sqlx::Result<()> {
    let disks: Vec<_> = s
        .disks
        .iter()
        .map(|d| {
            json!({
                "name": d.name,
                "used_bytes": d.used_bytes,
                "read_bytes_per_sec": d.read_bytes_per_sec,
                "write_bytes_per_sec": d.write_bytes_per_sec,
            })
        })
        .collect();
    let interfaces: Vec<_> = s
        .interfaces
        .iter()
        .map(|i| {
            json!({
                "name": i.name,
                "rx_bytes_per_sec": i.rx_bytes_per_sec,
                "tx_bytes_per_sec": i.tx_bytes_per_sec,
                "link_speed_bps": i.link_speed_bps, // null when unknown
            })
        })
        .collect();
    sqlx::query(
        "INSERT INTO samples (endpoint_id, collected_at, received_at, cpu_usage_percent,
                              memory_used_bytes, memory_total_bytes, disks, interfaces)
         VALUES ($1, $2, now(), $3, $4, $5, $6, $7)",
    )
    .bind(endpoint_id)
    .bind(collected_at(s.collected_at.as_ref()))
    .bind(s.cpu_usage_percent)
    .bind(to_bigint(s.memory_used_bytes))
    .bind(to_bigint(s.memory_total_bytes))
    .bind(json!(disks))
    .bind(json!(interfaces))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch_last_seen(pool: &PgPool, endpoint_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE endpoints SET last_seen_at = now() WHERE id = $1")
        .bind(endpoint_id)
        .execute(pool)
        .await?;
    Ok(())
}

fn without_timestamp(mut inv: Inventory) -> Inventory {
    inv.collected_at = None;
    inv
}

/// The Agent's timestamp; falls back to now if the Agent omitted or garbled it.
fn collected_at(ts: Option<&prost_types::Timestamp>) -> DateTime<Utc> {
    ts.and_then(|t| DateTime::from_timestamp(t.seconds, u32::try_from(t.nanos).ok()?))
        .unwrap_or_else(Utc::now)
}

fn to_bigint(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}
