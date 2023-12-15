use rustyrmm_proto::endpoint_registration::{
    registration_service_client::RegistrationServiceClient, ClientVer, Cpu, CpuInfo, Disk,
    DiskInfo, DiskType, EndpointRegistration, EndpointUpdate, Ip4Addr, Ip6Addr, MemInfo, NetInfo,
    NetInterface, OsInfo, UpdateStatus,
};
use rustyrmm_types::{ids::AssetId, registration_state::RegistrationState};

use crate::helpers::spawn_app;

const RUSTYRMM_VERSION: &str = env!("CARGO_PKG_VERSION");
const UNKNOWN: &str = "Unknown";

#[tokio::test]
async fn register_endpoint() {
    let state = spawn_app().await;

    let request = tonic::Request::new(EndpointRegistration {
        hostname: String::from("myhost"),
        system_serial_number: String::from("myserial"),
        system_sku_number: String::from("mysku"),
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

    let saved = sqlx::query!(r#"SELECT id AS "id: AssetId", hostname, system_serial_number, system_sku_number, reg_state AS "reg_state: RegistrationState" FROM endpoint"#)
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
        saved.system_serial_number.unwrap(),
        "myserial",
        "The id in the server response and the database match"
    );
    assert_eq!(
        saved.system_sku_number.unwrap(),
        "mysku",
        "The id in the server response and the database match"
    );
}

#[tokio::test]
async fn update_endpoint() {
    let state = spawn_app().await;

    let request = tonic::Request::new(EndpointRegistration {
        hostname: String::from("myhost"),
        system_serial_number: String::from("myserial"),
        system_sku_number: String::from("mysku"),
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let app_address = state.app_address;
    println!("gRPC Client trying to connect: {}", app_address.clone());
    let mut grpc_client = RegistrationServiceClient::connect(app_address.clone())
        .await
        .expect("Failed to launch gRPC client");
    let response = grpc_client
        .register_endpoint(request)
        .await
        .expect("gRPC call to register endpoint failed");
    let response = response.into_inner();
    let rusty_id = response.clone().id.unwrap();
    let asset_id = AssetId::from(rusty_id.clone());

    let request = tonic::Request::new(EndpointUpdate {
        id: Some(rusty_id),
        hostname: String::from("myhost"),
        system_serial_number: String::from("myserial"),
        system_sku_number: String::from("mysku"),
        os: Some(OsInfo {
            full_name: String::from("Ubuntu 22.04"),
            family: String::from("Debian"),
            version: String::from("22.04"),
            kernel_version: String::from("5.0.0"),
            virt_system: UNKNOWN.to_string(),
            virt_role: UNKNOWN.to_string(),
            tz: String::from("EAT Utc+03:00"),
            machine_id: String::from("OS-generated-uuid"),
        }),
        cpu: Some(CpuInfo {
            core_count: 4,
            thread_count: 8,
            cpus: vec![
                Cpu {
                    name: String::from("Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz"),
                    vendor_id: String::from("GenuineIntel"),
                    brand: String::from("Intel"),
                    frequency: String::from("1.80GHz"),
                },
                Cpu {
                    name: String::from("Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz"),
                    vendor_id: String::from("GenuineIntel"),
                    brand: String::from("Intel"),
                    frequency: String::from("1.80GHz"),
                },
                Cpu {
                    name: String::from("Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz"),
                    vendor_id: String::from("GenuineIntel"),
                    brand: String::from("Intel"),
                    frequency: String::from("1.80GHz"),
                },
                Cpu {
                    name: String::from("Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz"),
                    vendor_id: String::from("GenuineIntel"),
                    brand: String::from("Intel"),
                    frequency: String::from("1.80GHz"),
                },
            ],
        }),
        memory: Some(MemInfo {
            total: 16000000000,
            used: 9000000000,
        }),
        disks: Some(DiskInfo {
            disks: vec![Disk {
                name: String::from("Some Hdd"),
                disk_type: DiskType::TypeSsd as i32,
                filesystem: String::from("EXT4"),
                mount_point: String::from("/"),
                size: 1000000000000,
                free: 20000000000,
            }],
        }),
        net: Some(NetInfo {
            ifs: vec![NetInterface {
                name: String::from("eth0"),
                mac: String::from("00:1a:2b:3c:4d:5e"),
                ip4: vec![Ip4Addr {
                    ip: String::from("127.0.0.1"),
                }],
                ip6: vec![Ip6Addr {
                    ip: String::from("[::1]"),
                }],
            }],
        }),
        updates: Some(UpdateStatus {
            security: 0,
            regular: 10,
        }),
        client_version: Some(ClientVer {
            version: String::from(RUSTYRMM_VERSION),
        }),
    });
    let response = grpc_client
        .update_endpoint(request)
        .await
        .expect("gRPC call to update endpoint failed");
    let response = response.into_inner();

    let saved_update =
        sqlx::query!(r#"SELECT id AS "id: AssetId", os_name, os_ver, kernel_ver FROM os_info"#)
            .fetch_one(&state.db_pool)
            .await
            .expect("failed to retrieve saved endpoint");

    assert_eq!(
        saved_update.id, asset_id,
        "The id in the server response and the id from the database match"
    );
    assert_eq!(
        saved_update.os_name.unwrap(),
        "Ubuntu 22.04",
        "The OS name was saved"
    );
    assert_eq!(
        saved_update.os_ver.unwrap(),
        "22.04",
        "The OS version was saved"
    );
    assert_eq!(
        saved_update.kernel_ver.unwrap(),
        "5.0.0",
        "The kernel version was saved"
    );
}
