use std::sync::{Arc, Mutex};

use rustyrmm_server::startup;
use sqlx::{Connection, Executor, PgConnection, PgPool, Pool, Postgres};
use uuid::Uuid;

pub struct TestState {
    pub app_address: String,
    pub db_pool: Pool<Postgres>,
    pub port: u16,
}

#[derive(Debug)]
struct PortProvider {
    inner: Arc<Mutex<u16>>,
}

lazy_static! {
    static ref PORT: PortProvider = PortProvider::new();
}

pub async fn spawn_app() -> TestState {
    let port = PORT.get();
    let addr_str = format!("127.0.0.1:{}", port);
    let addr = startup::build_address(addr_str.clone()).unwrap();
    let mut settings = startup::get_settings();
    settings.database.database_name = Uuid::new_v4().to_string();
    let mut pg_connection = PgConnection::connect(&settings.database.postgres_url())
        .await
        .expect("failed to connect to postgres");
    pg_connection
        .execute(format!(r#"CREATE DATABASE "{}";"#, settings.database.database_name).as_str())
        .await
        .expect("failed to create database");

    let db_pool = PgPool::connect(&settings.database.database_url())
        .await
        .expect("failed to create connection pool to database");
    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await
        .expect("failed to migrate database");

    let database_url = settings.database.database_url();
    let _ = tokio::spawn(async move { startup::serve(addr, database_url).await });

    TestState {
        app_address: format!("http://127.0.0.1:{}", port),
        port,
        db_pool,
    }
}

impl PortProvider {
    pub fn new() -> PortProvider {
        Self {
            inner: Arc::new(Mutex::new(61000)),
        }
    }

    pub fn get(&self) -> u16 {
        let mut data = self.inner.lock().unwrap();
        *data -= 1;

        data.clone()
    }
}
