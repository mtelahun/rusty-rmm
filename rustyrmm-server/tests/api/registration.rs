use rustyrmm_proto::endpoint_registration::{
    registration_service_client::RegistrationServiceClient, EndpointRegistration,
};
use rustyrmm_types::{ids::AssetId, registration_state::RegistrationState};

use crate::helpers::spawn_app;

#[tokio::test]
async fn register_endpoint() {
    let state = spawn_app().await;

    let request = tonic::Request::new(EndpointRegistration {
        hostname: String::from("myhost"),
        system_uuid: String::from("mysystemid"),
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    println!("gRPC Client trying to connect: {}", state.app_address);
    let mut grpc_client = RegistrationServiceClient::connect(state.app_address)
        .await
        .expect("Failed to launch gRPC client");
    let response = grpc_client
        .register_endpoint(request)
        .await
        .expect("gRPC call to register endpoint failed");
    let response = response.into_inner();
    let id = response.clone().id.unwrap();

    let saved = sqlx::query!(r#"SELECT id AS "id: AssetId", hostname, system_id, reg_state AS "reg_state: RegistrationState" FROM endpoint"#)
        .fetch_one(&state.db_pool)
        .await
        .expect("failed to retrieve saved endpoint");

    assert_eq!(
        saved.id,
        AssetId::from(id),
        "The id in the server response and the database match"
    );
    assert_eq!(
        saved.hostname.unwrap(),
        "myhost",
        "The id in the server response and the database match"
    );
    assert_eq!(
        saved.system_id.unwrap(),
        "mysystemid",
        "The id in the server response and the database match"
    );
}
