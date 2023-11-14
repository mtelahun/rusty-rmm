use std::{env, error::Error, net::SocketAddr, path::PathBuf};

use rustyrmm_proto::endpoint_registration::registration_service_server::RegistrationServiceServer;
use tonic::transport::Server;

use crate::{
    cli::parse_cli,
    configuration::{get_configuration, Settings},
    db,
    server::EndPt,
};

pub fn get_settings() -> Settings {
    let cli = parse_cli();
    let mut config_file_path: Option<PathBuf> = None;
    if let Some(path) = cli.config {
        config_file_path = Some(path);
    } else if env::var("RUSTYRMM_SERVER_CONFIG_PATH").is_ok() {
        config_file_path = Some(PathBuf::from(
            env::var("RUSTYRMM_SERVER_CONFIG_PATH").unwrap(),
        ));
    }
    let settings = get_configuration(config_file_path, cli.no_config);
    if settings.is_err() {
        panic!("unable to read configuration: {:?}", settings.err());
    }

    settings.unwrap()
}

pub fn build_address(address: String) -> Result<SocketAddr, Box<dyn Error>> {
    let addr: SocketAddr = address.parse()?;

    Ok(addr)
}

pub async fn serve(addr: SocketAddr, database_url: String) {
    let db_pool = db::create_pool(&database_url)
        .map_err(|e| eprintln!("{}", e))
        .unwrap();
    let endpoint = EndPt::new(db_pool);
    println!("Server listening on: {}", addr);

    let _ = Server::builder()
        .add_service(RegistrationServiceServer::new(endpoint))
        .serve(addr)
        .await
        .map_err(|e| eprintln!("{}", e));
}
