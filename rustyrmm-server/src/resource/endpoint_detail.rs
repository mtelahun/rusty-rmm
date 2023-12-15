use tokio_postgres::Row;

use rustyrmm_types::ids::AssetId;

use super::Resource;

#[derive(Debug, Default)]
pub struct EndpointDetail {
    pub id: AssetId,
    pub hostname: String,
    pub system_sku_number: String,
    pub system_serial_number: String,
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
            hostname: value.get("hostname"),
            system_serial_number: value.get("system_serial_number"),
            system_sku_number: value.get("system_sku_number"),
            os: value.get("os"),
            cpu: value.get("cpu"),
            mem: value.get("mem"),
            disk: value.get("disk"),
            net: value.get("net"),
            client_ver: value.get("client_ver"),
        }
    }
}
