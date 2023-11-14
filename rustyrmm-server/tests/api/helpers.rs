use std::sync::{Arc, Mutex};

use rustyrmm_proto::endpoint_registration::registration_service_client::RegistrationServiceClient;
use rustyrmm_server::startup;
use sqlx::{Connection, Executor, PgConnection, PgPool, Pool, Postgres};
use tokio_postgres::NoTls;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct TestState {
    pub app_address: String,
    pub grpc_client: RegistrationServiceClient<Channel>,
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

pub async fn connect_to_db(db_name: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(
        &format!(
            "host=localhost user=postgres password=password dbname={}",
            db_name
        ),
        NoTls,
    )
    .await
    .expect("Unable to connect to test database");
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e)
        }
    });

    client
}

pub async fn spawn_app() -> TestState {
    let server = "127.0.0.1";
    let port = PORT.get();
    let addr = startup::build_address(format!("127.0.0.1:{}", port)).unwrap();
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

    let _ = tokio::spawn(startup::serve(addr, settings.database.database_url()));

    let grpc_client = RegistrationServiceClient::connect(format!("http://{}:{}", server, port))
        .await
        .expect("Failed to launch gRPC client");

    TestState {
        app_address: format!("http://127.0.0.1:{}", port),
        port,
        grpc_client,
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
