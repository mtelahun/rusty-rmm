//! Stores what Agents send over their Sessions.

use chrono::{DateTime, Utc};
use prost::Message;
use rustyrmm_proto::common::v1::{Inventory, PendingUpdates, Sample};
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
    .bind(agent_time(inv.collected_at.as_ref()))
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

/// Replaces the Endpoint's pending Updates with a known list: new keys get `first_seen_at = now()`,
/// existing keys keep theirs, keys no longer listed are deleted. An unknown list changes nothing.
pub async fn pending_updates(
    pool: &PgPool,
    endpoint_id: Uuid,
    list: &PendingUpdates,
) -> sqlx::Result<()> {
    if !list.known {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for u in &list.updates {
        sqlx::query(
            "INSERT INTO pending_updates (endpoint_id, install_key, title, security,
                                          installed_version, new_version, severity, advisory_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (endpoint_id, install_key) DO UPDATE SET
                 title = EXCLUDED.title, security = EXCLUDED.security,
                 installed_version = EXCLUDED.installed_version,
                 new_version = EXCLUDED.new_version, severity = EXCLUDED.severity,
                 advisory_id = EXCLUDED.advisory_id",
        )
        .bind(endpoint_id)
        .bind(&u.install_key)
        .bind(&u.title)
        .bind(u.security)
        .bind(&u.installed_version)
        .bind(&u.new_version)
        .bind(&u.severity)
        .bind(&u.advisory_id)
        .execute(&mut *tx)
        .await?;
    }
    let keys: Vec<&str> = list
        .updates
        .iter()
        .map(|u| u.install_key.as_str())
        .collect();
    sqlx::query(
        "DELETE FROM pending_updates WHERE endpoint_id = $1 AND NOT (install_key = ANY($2))",
    )
    .bind(endpoint_id)
    .bind(&keys)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE endpoints SET update_metadata_refreshed_at = $2 WHERE id = $1")
        .bind(endpoint_id)
        .bind(
            list.metadata_refreshed_at
                .as_ref()
                .map(|t| agent_time(Some(t))),
        )
        .execute(&mut *tx)
        .await?;
    tx.commit().await
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
    .bind(agent_time(s.collected_at.as_ref()))
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
pub fn agent_time(ts: Option<&prost_types::Timestamp>) -> DateTime<Utc> {
    ts.and_then(|t| DateTime::from_timestamp(t.seconds, u32::try_from(t.nanos).ok()?))
        .unwrap_or_else(Utc::now)
}

fn to_bigint(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyrmm_proto::common::v1::PendingUpdate;

    async fn endpoint(pool: &PgPool) -> Uuid {
        let env = Uuid::now_v7();
        let ep = Uuid::now_v7();
        sqlx::query("INSERT INTO environments (id, name) VALUES ($1, $1::text)")
            .bind(env)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO endpoints (id, environment_id, hostname) VALUES ($1, $2, 'h')")
            .bind(ep)
            .bind(env)
            .execute(pool)
            .await
            .unwrap();
        ep
    }

    fn list(updates: &[(&str, &str)]) -> PendingUpdates {
        PendingUpdates {
            known: true,
            updates: updates
                .iter()
                .map(|(key, version)| PendingUpdate {
                    install_key: key.to_string(),
                    title: key.to_string(),
                    new_version: version.to_string(),
                    ..Default::default()
                })
                .collect(),
            metadata_refreshed_at: None,
        }
    }

    async fn rows(pool: &PgPool, ep: Uuid) -> Vec<(String, String, DateTime<Utc>)> {
        sqlx::query_as(
            "SELECT install_key, new_version, first_seen_at FROM pending_updates
             WHERE endpoint_id = $1 ORDER BY install_key",
        )
        .bind(ep)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn first_seen_is_kept_across_reports_and_a_version_bump(pool: PgPool) {
        let ep = endpoint(&pool).await;
        pending_updates(&pool, ep, &list(&[("curl", "1"), ("vim", "1")]))
            .await
            .unwrap();
        let first = rows(&pool, ep).await;
        pending_updates(&pool, ep, &list(&[("curl", "1"), ("vim", "2")]))
            .await
            .unwrap();
        let second = rows(&pool, ep).await;
        assert_eq!(first[0], second[0], "same report keeps the row");
        assert_eq!(second[1].1, "2", "the version updates");
        assert_eq!(first[1].2, second[1].2, "first seen survives the bump");
    }

    #[sqlx::test]
    async fn an_update_no_longer_listed_is_deleted(pool: PgPool) {
        let ep = endpoint(&pool).await;
        pending_updates(&pool, ep, &list(&[("curl", "1"), ("vim", "1")]))
            .await
            .unwrap();
        pending_updates(&pool, ep, &list(&[("vim", "1")]))
            .await
            .unwrap();
        let keys: Vec<_> = rows(&pool, ep).await.into_iter().map(|r| r.0).collect();
        assert_eq!(keys, ["vim"]);
    }

    #[sqlx::test]
    async fn an_unknown_report_keeps_the_last_list_and_refresh_time(pool: PgPool) {
        let ep = endpoint(&pool).await;
        let mut known = list(&[("curl", "1")]);
        known.metadata_refreshed_at = Some(prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 0,
        });
        pending_updates(&pool, ep, &known).await.unwrap();
        let before = rows(&pool, ep).await;
        pending_updates(&pool, ep, &PendingUpdates::default())
            .await
            .unwrap();
        assert_eq!(rows(&pool, ep).await, before);
        let refreshed: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT update_metadata_refreshed_at FROM endpoints WHERE id = $1")
                .bind(ep)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(refreshed.unwrap().timestamp(), 1_700_000_000);
    }

    #[sqlx::test]
    async fn a_known_empty_list_deletes_all_rows(pool: PgPool) {
        let ep = endpoint(&pool).await;
        pending_updates(&pool, ep, &list(&[("curl", "1"), ("vim", "1")]))
            .await
            .unwrap();
        pending_updates(&pool, ep, &list(&[])).await.unwrap();
        assert!(rows(&pool, ep).await.is_empty());
    }

    #[sqlx::test]
    async fn a_list_adds_no_inventory_row_and_goes_with_its_endpoint(pool: PgPool) {
        let ep = endpoint(&pool).await;
        pending_updates(&pool, ep, &list(&[("curl", "1")]))
            .await
            .unwrap();
        let inventories: i64 = sqlx::query_scalar("SELECT count(*) FROM inventories")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(inventories, 0);
        sqlx::query("DELETE FROM endpoints WHERE id = $1")
            .bind(ep)
            .execute(&pool)
            .await
            .unwrap();
        assert!(rows(&pool, ep).await.is_empty(), "cascade");
    }
}
