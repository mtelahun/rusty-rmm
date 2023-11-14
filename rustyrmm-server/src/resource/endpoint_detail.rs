use tokio_postgres::Row;

use rustyrmm_types::ids::{AssetId, MachineId};

use super::Resource;

#[derive(Debug, Default)]
pub struct EndpointDetail {
    pub id: AssetId,
    pub machine_id: MachineId,
    pub hostname: String,
    pub os: AssetId,
    pub cpu: AssetId,
    pub mem: AssetId,
    pub disk: AssetId,
    pub net: AssetId,
    pub client_ver: AssetId,
}

impl Resource for EndpointDetail {
    const NAME: &'static str = "endpoint_detail";
}

impl From<Row> for EndpointDetail {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
            machine_id: value.get("system_id"),
            hostname: value.get("hostname"),
            os: value.get("os"),
            cpu: value.get("cpu"),
            mem: value.get("mem"),
            disk: value.get("disk"),
            net: value.get("net"),
            client_ver: value.get("client_ver"),
        }
    }
}
