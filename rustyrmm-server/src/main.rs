mod agent_service;
mod ca;
mod db;
mod operators;
mod pairing;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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
        } => {
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
