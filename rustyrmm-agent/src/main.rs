use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rustyrmm_agent::{pairing, session, state};

#[derive(Parser)]
#[command(version, about = "RustyRMM Agent")]
struct Cli {
    /// Where the Agent keeps its key, certificate and Server details.
    #[arg(long, global = true, default_value_os_t = state::default_dir())]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pair this Endpoint with a Server using a Pairing Token.
    Pair {
        pairing_token: String,
        /// The Server's Agent port, as host:port.
        #[arg(long)]
        server: String,
        /// SHA-256 fingerprint of the Server's Agent CA, from the Pairing one-liner.
        #[arg(long)]
        ca_fingerprint: String,
    },
    /// Keep a Session open with the Server (what the service runs).
    Run,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Pair {
            pairing_token,
            server,
            ca_fingerprint,
        } => {
            let id =
                pairing::pair(&cli.state_dir, &server, &pairing_token, &ca_fingerprint).await?;
            println!("Paired as Endpoint {id}");
        }
        Command::Run => session::run(&cli.state_dir).await?,
    }
    Ok(())
}
