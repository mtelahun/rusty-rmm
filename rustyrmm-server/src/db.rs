use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Connects to Postgres and applies any pending migrations.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new().connect(database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
