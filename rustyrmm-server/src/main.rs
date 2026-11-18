mod agent_service;
mod ca;
mod db;
mod ingest;
mod operators;
mod pairing;
mod sessions;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rustyrmm_proto::agent::v1::agent_service_server::AgentServiceServer;

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
        /// DNS names or IPs Agents use to reach the Agent port (repeatable).
        #[arg(long = "agent-name", required = true)]
        agent_names: Vec<String>,
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
            state_dir,
            heartbeat_secs,
            offline_after_secs,
            sample_secs,
            inventory_secs,
        } => {
            let settings = sessions::Settings {
                heartbeat_interval: Duration::from_secs(heartbeat_secs),
                offline_after: Duration::from_secs(offline_after_secs),
                sample_interval: Duration::from_secs(sample_secs),
                inventory_interval: Duration::from_secs(inventory_secs),
                ..Default::default()
            };
            let pool = db::connect(&database_url).await?;
            let ca = Arc::new(ca::Ca::load_or_create(&state_dir)?);
            println!(
                "Agent CA fingerprint (SHA-256): {}",
                ca.fingerprint_sha256()
            );
            println!("Agent port listening on {agent_listen}");
            tonic::transport::Server::builder()
                .tls_config(agent_service::tls_config(&ca, agent_names)?)?
                .add_service(AgentServiceServer::new(agent_service::AgentApi {
                    pool,
                    ca,
                    sessions: Default::default(),
                    settings,
                }))
                .serve(agent_listen)
                .await?;
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
