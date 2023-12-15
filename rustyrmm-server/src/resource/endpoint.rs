use tokio_postgres::Row;

use rustyrmm_types::{ids::AssetId, registration_state::RegistrationState};

use super::Resource;

#[derive(Debug, Default)]
pub struct Endpoint {
    pub id: AssetId,
    pub system_serial_number: String,
    pub system_sku_number: String,
    pub hostname: String,
    pub reg_state: RegistrationState,
}

impl Resource for Endpoint {
    const NAME: &'static str = "endpoint";
}

impl From<Row> for Endpoint {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
            system_serial_number: value.get("system_serial_number"),
            system_sku_number: value.get("system_sku_number"),
            hostname: value.get("hostname"),
            reg_state: value.get("reg_state"),
        }
    }
}
