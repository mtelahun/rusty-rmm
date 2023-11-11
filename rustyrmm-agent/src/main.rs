use ::clap::Parser;
use rustyrmm_proto::endpoint_registration::{
    registration_service_client::RegistrationServiceClient, ClientVer, EndpointRegistration,
    EndpointUpdate, RustyRmmId,
};
use sysinfo::System;

use crate::{
    error::AgentError,
    system_info::{
        get_cpu, get_disk, get_hostname, get_memory, get_os, get_system_id, init_system,
    },
};

mod error;
mod system_info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

    let request = tonic::Request::new(EndpointRegistration {
        hostname: get_hostname(&state.system).map_err(AgentError::Internal)?,
        system_uuid: get_system_id().map_err(AgentError::Internal)?.to_string(),
    });

    let response = client.register_endpoint(request).await?;
    let response = response.into_inner();
    let id = response.clone().id.unwrap_or(RustyRmmId::default());

    println!("RESPONSE={:?}", response.clone());

    let request = tonic::Request::new(EndpointUpdate {
        id: Some(id),
        hostname: get_hostname(&state.system).map_err(AgentError::Internal)?,
        system_uuid: get_system_id().map_err(AgentError::Internal)?.to_string(),
        os: Some(get_os(&state.system).map_err(AgentError::Internal)?),
        cpu: Some(get_cpu(&state.system)),
        memory: Some(get_memory(&state.system)),
        disks: Some(get_disk(&state.system)),
        ips: todo!(),
        updates: todo!(),
        client_version: Some(ClientVer {
            version: String::from(VERSION),
        }),
    });

    let response = client.update_endpoint(request).await?;
    println!("RESPONSE={:?}", response.into_inner());

    Ok(())
}

#[derive(Debug, Default)]
struct State {
    system: System,
}

impl State {
    fn new() -> State {
        Self {
            system: init_system(),
        }
    }
}
