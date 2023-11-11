use std::env;

use clap::Parser;
use rustyrmm_proto::endpoint_registration::registration_service_server::RegistrationServiceServer;
use rustyrmm_server::{db, server::EndPt};
use tonic::transport::Server;

#[derive(Parser)]
#[command(author, version)]
#[command(about = "echo-server - a simple echo microservice", long_about = None)]
struct ServerCli {
    #[arg(short = 's', long = "server", default_value = "127.0.0.1")]
    server: String,

    #[arg(short = 'p', long = "port", default_value = "50052")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = ServerCli::parse();
    let addr = format!("{}:{}", cli.server, cli.port).parse()?;
    let db_pool = db::create_pool(&env::var("DATABASE_URL").unwrap_or(String::from("")))?;
    let endpoint = EndPt::new(db_pool);
    println!("Server listening on: {}", addr);

    Server::builder()
        .add_service(RegistrationServiceServer::new(endpoint))
        .serve(addr)
        .await?;

    Ok(())
}
