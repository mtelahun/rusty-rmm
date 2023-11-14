use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(author, version)]
#[command(about = "echo-server - a simple echo microservice", long_about = None)]
pub struct ServerCli {
    #[arg(short = 'c', long = "configuration")]
    pub config: Option<PathBuf>,

    #[arg(short = 'C', long = "no-config")]
    pub no_config: bool,

    #[arg(short = 's', long = "server", default_value = "127.0.0.1")]
    pub server: Option<String>,

    #[arg(short = 'p', long = "port", default_value = "50052")]
    pub port: Option<u16>,
}

pub fn parse_cli() -> ServerCli {
    ServerCli::parse()
}
