use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rustyrmm_proto::agent::v1::agent_service_server::AgentServiceServer;
use rustyrmm_server::{agent_service, ca, db, jobs, operator_service, operators, sessions};

#[derive(Parser)]
#[command(version, about = "RustyRMM Server")]
struct Cli {
    /// Postgres connection URL.
    #[arg(long, env = "DATABASE_URL", global = true, hide_env_values = true)]
    database_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Server.
    Serve {
        /// Address for the Agent port (mTLS).
        #[arg(long, default_value = "0.0.0.0:7443")]
        agent_listen: SocketAddr,
        /// DNS names or IPs Agents use to reach the Agent port (repeatable). The first one,
        /// with the Agent port's port, goes into Pairing one-liners.
        #[arg(long = "agent-name", required = true)]
        agent_names: Vec<String>,
        /// Address for the Operator port (CLI and web UI).
        #[arg(long, default_value = "0.0.0.0:8443")]
        operator_listen: SocketAddr,
        /// PEM certificate chain for the Operator port (from a public CA).
        #[arg(long)]
        operator_cert: PathBuf,
        /// PEM private key for the Operator port's certificate.
        #[arg(long)]
        operator_key: PathBuf,
        /// Directory for the CA certificate and its root-only key.
        #[arg(long, default_value = "/var/lib/rustyrmm")]
        state_dir: PathBuf,
        /// Seconds between Agent heartbeats.
        #[arg(long, default_value_t = 30)]
        heartbeat_secs: u64,
        /// Seconds without any Agent message before its Endpoint goes Offline.
        #[arg(long, default_value_t = 90)]
        offline_after_secs: u64,
        /// Seconds between Samples.
        #[arg(long, default_value_t = 30)]
        sample_secs: u64,
        /// Seconds between Inventory change checks.
        #[arg(long, default_value_t = 3600)]
        inventory_secs: u64,
        /// Days to keep raw 30-second Samples.
        #[arg(long, default_value_t = 7)]
        sample_retention_days: u64,
        /// Days to keep 5-minute Sample rollups.
        #[arg(long, default_value_t = 90)]
        rollup_retention_days: u64,
    },
    /// Manage Operators (run on the Server host).
    Operator {
        #[command(subcommand)]
        command: OperatorCommand,
    },
}

#[derive(Subcommand)]
enum OperatorCommand {
    /// Create an Operator; prompts for the password.
    Add { username: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let database_url = cli
        .database_url
        .ok_or_else(|| anyhow::anyhow!("set --database-url or DATABASE_URL"))?;

    match cli.command {
        Command::Serve {
            agent_listen,
            agent_names,
            operator_listen,
            operator_cert,
            operator_key,
            state_dir,
            heartbeat_secs,
            offline_after_secs,
            sample_secs,
            inventory_secs,
            sample_retention_days,
            rollup_retention_days,
        } => {
            const DAY: u64 = 86_400;
            let retention = jobs::Retention {
                raw_samples: Duration::from_secs(sample_retention_days * DAY),
                rollups: Duration::from_secs(rollup_retention_days * DAY),
            };
            let settings = sessions::Settings {
                heartbeat_interval: Duration::from_secs(heartbeat_secs),
                offline_after: Duration::from_secs(offline_after_secs),
                sample_interval: Duration::from_secs(sample_secs),
                inventory_interval: Duration::from_secs(inventory_secs),
                ..Default::default()
            };
            let pool = db::connect(&database_url).await?;
            let ca = Arc::new(ca::Ca::load_or_create(&state_dir)?);
            let sessions = Arc::new(sessions::Sessions::default());
            tokio::spawn(jobs::run(
                pool.clone(),
                retention,
                sessions.clone(),
                chrono::Utc::now(),
            ));
            println!(
                "Agent CA fingerprint (SHA-256): {}",
                ca.fingerprint_sha256()
            );
            let operator_api = operator_service::OperatorApi {
                pool: pool.clone(),
                sessions: sessions.clone(),
                agent_address: format!("{}:{}", agent_names[0], agent_listen.port()),
                ca_fingerprint: ca.fingerprint_sha256(),
            };
            let operator_tls = operator_service::tls_config(
                &std::fs::read_to_string(&operator_cert)?,
                &std::fs::read_to_string(&operator_key)?,
            );
            let agent_server = tonic::transport::Server::builder()
                .tls_config(agent_service::tls_config(&ca, agent_names)?)?
                .add_service(AgentServiceServer::new(agent_service::AgentApi {
                    pool,
                    ca,
                    sessions,
                    settings,
                }))
                .serve(agent_listen);
            let operator_listener = tokio::net::TcpListener::bind(operator_listen).await?;
            println!("Agent port listening on {agent_listen}");
            println!("Operator port listening on {operator_listen}");
            tokio::try_join!(
                async { agent_server.await.map_err(anyhow::Error::from) },
                operator_service::serve(operator_api, operator_tls, operator_listener),
            )?;
        }
        Command::Operator {
            command: OperatorCommand::Add { username },
        } => {
            let password = rpassword::prompt_password("Password: ")?;
            if rpassword::prompt_password("Confirm password: ")? != password {
                anyhow::bail!("passwords do not match");
            }
            let pool = db::connect(&database_url).await?;
            let id = operators::add_operator(&pool, &username, &password).await?;
            println!("Created Operator {username} ({id})");
        }
    }
    Ok(())
}
