use ::clap::Parser;
use rustyrmm_proto::endpoint_registration::{
    registration_service_client::RegistrationServiceClient, ClientVer, EndpointRegistration,
    EndpointUpdate, RustyRmmId,
};
use system_info::SystemInformation;

use crate::error::AgentError;

mod error;
mod system_info;

const RUSTYRMM_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(author, version)]
#[command(about = "echo - a simple echo microservice client", long_about = None)]
struct ClientCli {
    #[arg(short = 's', long = "server", default_value = "127.0.0.1")]
    server: String,

    #[arg(short = 'p', long = "port", default_value = "50052")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), AgentError> {
    let cli = ClientCli::parse();
    let mut client =
        RegistrationServiceClient::connect(format!("http://{}:{}", cli.server, cli.port)).await?;
    let state = State::new();
    let system = &state.system;

    let request = tonic::Request::new(EndpointRegistration {
        hostname: system.get_hostname().map_err(AgentError::Internal)?,
        system_serial_number: "SERIAL".to_string(),
        system_sku_number: "SKU".to_string(),
    });

    let response = client.register_endpoint(request).await?;
    let response = response.into_inner();
    let id = response.clone().id.unwrap_or(RustyRmmId::default());

    println!("RESPONSE={:?}", response.clone());

    let request = tonic::Request::new(EndpointUpdate {
        id: Some(id),
        hostname: system.get_hostname().map_err(AgentError::Internal)?,
        system_serial_number: "SERIAL".to_string(),
        system_sku_number: "SKU".to_string(),
        os: Some(system.get_os().map_err(AgentError::Internal)?),
        cpu: Some(system.get_cpu()),
        memory: Some(system.get_memory()),
        disks: Some(system.get_disk()),
        net: Some(system.get_network()),
        updates: Some(system.get_update()),
        client_version: Some(ClientVer {
            version: String::from(RUSTYRMM_VERSION),
        }),
    });

    let response = client.update_endpoint(request).await?;
    println!("RESPONSE={:?}", response.into_inner());

    Ok(())
}

#[derive(Debug, Default)]
struct State {
    system: SystemInformation,
}

impl State {
    fn new() -> State {
        Self {
            system: SystemInformation::new(),
        }
    }
}
