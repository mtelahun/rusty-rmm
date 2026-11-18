//! Background jobs run on a timer inside the Server: Command timeouts, Sample rollups, retention.

use std::time::Duration;

use sqlx::PgPool;

use crate::commands;

#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub raw_samples: Duration,
    pub rollups: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            raw_samples: Duration::from_secs(7 * 86_400),
            rollups: Duration::from_secs(90 * 86_400),
        }
    }
}

/// How long past a Command's own timeout to wait for its result (covers an Agent reconnecting
/// and resending) before marking it "outcome unknown".
// shortcut: fixed at 5 minutes; becomes a setting with the Command execution limits decision.
pub const OUTCOME_UNKNOWN_GRACE: Duration = Duration::from_secs(300);

/// Runs forever: the Command sweep every minute; rollups and retention every 5 minutes.
pub async fn run(pool: PgPool, retention: Retention) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    for n in 0u64.. {
        tick.tick().await;
        report(
            "command timeouts",
            commands::mark_outcome_unknown(&pool, OUTCOME_UNKNOWN_GRACE).await,
        );
        if n % 5 == 0 {
            report("sample rollup", rollup_samples(&pool).await);
            report("retention", apply_retention(&pool, retention).await);
        }
    }
}

fn report(job: &str, r: sqlx::Result<u64>) {
    if let Err(e) = r {
        // shortcut: stderr until the Server adopts structured logging.
        eprintln!("{job} job failed: {e}");
    }
}

/// (Re)computes 5-minute rollups for completed buckets in the last hour. Recomputing the hour
/// picks up late-arriving Samples; the upsert keeps it idempotent.
pub async fn rollup_samples(pool: &PgPool) -> sqlx::Result<u64> {
    let done = sqlx::query(
        r#"
        WITH bounds AS (
            SELECT date_bin('5 minutes', now(), TIMESTAMPTZ 'epoch') AS hi
        ),
        w AS (
            SELECT s.*, date_bin('5 minutes', s.collected_at, TIMESTAMPTZ 'epoch') AS bucket
            FROM samples s, bounds
            WHERE s.collected_at >= bounds.hi - interval '1 hour' AND s.collected_at < bounds.hi
        ),
        base AS (
            SELECT endpoint_id, bucket,
                   avg(cpu_usage_percent)::real    AS cpu_avg,
                   max(cpu_usage_percent)          AS cpu_max,
                   avg(memory_used_bytes)::bigint  AS mem_avg,
                   max(memory_used_bytes)          AS mem_max
            FROM w GROUP BY endpoint_id, bucket
        ),
        disks AS (
            SELECT endpoint_id, bucket, jsonb_agg(jsonb_build_object(
                       'name', name,
                       'used_bytes_max', used_max,
                       'read_bytes_per_sec_avg', r_avg, 'read_bytes_per_sec_max', r_max,
                       'write_bytes_per_sec_avg', w_avg, 'write_bytes_per_sec_max', w_max)
                       ORDER BY name) AS disks
            FROM (
                SELECT endpoint_id, bucket, d->>'name' AS name,
                       max((d->>'used_bytes')::bigint)                    AS used_max,
                       avg((d->>'read_bytes_per_sec')::numeric)::bigint   AS r_avg,
                       max((d->>'read_bytes_per_sec')::bigint)            AS r_max,
                       avg((d->>'write_bytes_per_sec')::numeric)::bigint  AS w_avg,
                       max((d->>'write_bytes_per_sec')::bigint)           AS w_max
                FROM w, jsonb_array_elements(w.disks) d
                GROUP BY endpoint_id, bucket, d->>'name'
            ) per_disk
            GROUP BY endpoint_id, bucket
        ),
        ifaces AS (
            SELECT endpoint_id, bucket, jsonb_agg(jsonb_build_object(
                       'name', name,
                       'rx_bytes_per_sec_avg', rx_avg, 'rx_bytes_per_sec_max', rx_max,
                       'tx_bytes_per_sec_avg', tx_avg, 'tx_bytes_per_sec_max', tx_max,
                       'link_speed_bps', link_speed)
                       ORDER BY name) AS interfaces
            FROM (
                SELECT endpoint_id, bucket, i->>'name' AS name,
                       avg((i->>'rx_bytes_per_sec')::numeric)::bigint AS rx_avg,
                       max((i->>'rx_bytes_per_sec')::bigint)          AS rx_max,
                       avg((i->>'tx_bytes_per_sec')::numeric)::bigint AS tx_avg,
                       max((i->>'tx_bytes_per_sec')::bigint)          AS tx_max,
                       max((i->>'link_speed_bps')::bigint)            AS link_speed -- null if unknown
                FROM w, jsonb_array_elements(w.interfaces) i
                GROUP BY endpoint_id, bucket, i->>'name'
            ) per_iface
            GROUP BY endpoint_id, bucket
        )
        INSERT INTO sample_rollups (endpoint_id, bucket_start,
            cpu_usage_percent_avg, cpu_usage_percent_max,
            memory_used_bytes_avg, memory_used_bytes_max, disks, interfaces)
        SELECT b.endpoint_id, b.bucket, b.cpu_avg, b.cpu_max, b.mem_avg, b.mem_max,
               coalesce(d.disks, '[]'), coalesce(i.interfaces, '[]')
        FROM base b
        LEFT JOIN disks d USING (endpoint_id, bucket)
        LEFT JOIN ifaces i USING (endpoint_id, bucket)
        ON CONFLICT (endpoint_id, bucket_start) DO UPDATE SET
            cpu_usage_percent_avg = EXCLUDED.cpu_usage_percent_avg,
            cpu_usage_percent_max = EXCLUDED.cpu_usage_percent_max,
            memory_used_bytes_avg = EXCLUDED.memory_used_bytes_avg,
            memory_used_bytes_max = EXCLUDED.memory_used_bytes_max,
            disks = EXCLUDED.disks,
            interfaces = EXCLUDED.interfaces
        "#,
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Deletes data past its retention period. Returns the number of rows deleted.
pub async fn apply_retention(pool: &PgPool, retention: Retention) -> sqlx::Result<u64> {
    let mut deleted = 0;
    for (sql, keep) in [
        (
            "DELETE FROM samples WHERE received_at < now() - make_interval(secs => $1)",
            retention.raw_samples,
        ),
        (
            "DELETE FROM sample_rollups WHERE bucket_start < now() - make_interval(secs => $1)",
            retention.rollups,
        ),
        (
            "DELETE FROM commands WHERE created_at < now() - make_interval(secs => $1)",
            ONE_YEAR,
        ),
        // Old Inventory history goes, but each Endpoint's latest Inventory always stays.
        (
            "DELETE FROM inventories old
             WHERE old.received_at < now() - make_interval(secs => $1)
               AND EXISTS (SELECT 1 FROM inventories newer
                           WHERE newer.endpoint_id = old.endpoint_id AND newer.id > old.id)",
            ONE_YEAR,
        ),
        (
            "DELETE FROM operator_sessions WHERE expires_at < now() - make_interval(secs => $1)",
            Duration::ZERO,
        ),
    ] {
        deleted += sqlx::query(sql)
            .bind(keep.as_secs_f64())
            .execute(pool)
            .await?
            .rows_affected();
    }
    Ok(deleted)
}

const ONE_YEAR: Duration = Duration::from_secs(365 * 86_400);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

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

    /// Inserts a Sample `ago` before the start of the current 5-minute bucket.
    async fn sample(
        pool: &PgPool,
        ep: Uuid,
        ago: &str,
        cpu: f32,
        read_bps: i64,
        link: Option<i64>,
    ) {
        sqlx::query(
            "INSERT INTO samples (endpoint_id, collected_at, received_at, cpu_usage_percent,
                                  memory_used_bytes, memory_total_bytes, disks, interfaces)
             VALUES ($1, date_bin('5 minutes', now(), TIMESTAMPTZ 'epoch') - $2::interval,
                     now(), $3, 100, 1000, $4, $5)",
        )
        .bind(ep)
        .bind(ago)
        .bind(cpu)
        .bind(json!([{"name": "sda", "used_bytes": 5, "read_bytes_per_sec": read_bps, "write_bytes_per_sec": 0}]))
        .bind(json!([{"name": "eth0", "rx_bytes_per_sec": 10, "tx_bytes_per_sec": 20, "link_speed_bps": link}]))
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn rollup_averages_and_maxes_per_bucket_and_is_idempotent(pool: PgPool) {
        let ep = endpoint(&pool).await;
        // Two Samples in the bucket 10..5 minutes ago, one in the bucket before.
        sample(&pool, ep, "6 minutes", 10.0, 100, None).await;
        sample(&pool, ep, "9 minutes", 30.0, 300, None).await;
        sample(&pool, ep, "12 minutes", 50.0, 500, Some(1000)).await;
        // The current, incomplete bucket is not rolled up.
        sqlx::query(
            "INSERT INTO samples (endpoint_id, collected_at, received_at, cpu_usage_percent,
                                  memory_used_bytes, memory_total_bytes, disks, interfaces)
             VALUES ($1, now(), now(), 99, 1, 1, '[]', '[]')",
        )
        .bind(ep)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(rollup_samples(&pool).await.unwrap(), 2);
        assert_eq!(
            rollup_samples(&pool).await.unwrap(),
            2,
            "upsert, not duplicate"
        );

        let (avg, max, disks, ifaces): (f32, f32, serde_json::Value, serde_json::Value) =
            sqlx::query_as(
                "SELECT cpu_usage_percent_avg, cpu_usage_percent_max, disks, interfaces
                 FROM sample_rollups
                 WHERE bucket_start = date_bin('5 minutes', now(), TIMESTAMPTZ 'epoch') - interval '10 minutes'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((avg, max), (20.0, 30.0));
        assert_eq!(disks[0]["read_bytes_per_sec_avg"], 200);
        assert_eq!(disks[0]["read_bytes_per_sec_max"], 300);
        assert!(
            ifaces[0]["link_speed_bps"].is_null(),
            "unknown stays unknown"
        );
    }

    #[sqlx::test]
    async fn retention_deletes_old_data_but_keeps_latest_inventory(pool: PgPool) {
        let ep = endpoint(&pool).await;
        for age in ["400 days", "300 days"] {
            sqlx::query(
                "INSERT INTO inventories (endpoint_id, collected_at, received_at, inventory)
                 VALUES ($1, now() - $2::interval, now() - $2::interval, '')",
            )
            .bind(ep)
            .bind(age)
            .execute(&pool)
            .await
            .unwrap();
        }
        // A lone Inventory older than a year on another Endpoint is still its latest.
        let lone = endpoint(&pool).await;
        sqlx::query(
            "INSERT INTO inventories (endpoint_id, collected_at, received_at, inventory)
             VALUES ($1, now() - interval '500 days', now() - interval '500 days', '')",
        )
        .bind(lone)
        .execute(&pool)
        .await
        .unwrap();
        for (age, keep) in [("8 days", false), ("6 days", true)] {
            sqlx::query(
                "INSERT INTO samples (endpoint_id, collected_at, received_at, cpu_usage_percent,
                                      memory_used_bytes, memory_total_bytes, disks, interfaces)
                 VALUES ($1, now() - $2::interval, now() - $2::interval, $3, 0, 0, '[]', '[]')",
            )
            .bind(ep)
            .bind(age)
            .bind(if keep { 1.0f32 } else { 0.0 })
            .execute(&pool)
            .await
            .unwrap();
        }

        apply_retention(&pool, Retention::default()).await.unwrap();

        let inventories: Vec<Uuid> =
            sqlx::query_scalar("SELECT endpoint_id FROM inventories ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            inventories,
            vec![ep, lone],
            "the 400-day one went; latest ones stay"
        );
        let samples: Vec<f32> = sqlx::query_scalar("SELECT cpu_usage_percent FROM samples")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(samples, vec![1.0]);
    }

    #[sqlx::test]
    async fn commands_without_result_become_outcome_unknown(pool: PgPool) {
        let ep = endpoint(&pool).await;
        for (age, timeout) in [("10 minutes", 60), ("1 minute", 60)] {
            sqlx::query(
                "INSERT INTO commands (id, endpoint_id, endpoint_hostname, operator_username,
                                       interpreter, script, timeout_seconds, status, created_at)
                 VALUES ($1, $2, 'h', 'alice', 1, 'true', $3, 1, now() - $4::interval)",
            )
            .bind(Uuid::now_v7())
            .bind(ep)
            .bind(timeout)
            .bind(age)
            .execute(&pool)
            .await
            .unwrap();
        }
        // 10 minutes > 60s timeout + 5 min grace; 1 minute is still within it.
        let marked = commands::mark_outcome_unknown(&pool, OUTCOME_UNKNOWN_GRACE)
            .await
            .unwrap();
        assert_eq!(marked, 1);
    }
}
