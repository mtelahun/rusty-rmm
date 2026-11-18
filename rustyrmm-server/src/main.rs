mod db;
mod operators;

use clap::{Parser, Subcommand};

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
