use tokio_postgres::Row;

use rustyrmm_types::{
    ids::{AssetId, MachineId},
    registration_state::RegistrationState,
};

pub trait Resource {
    const NAME: &'static str;
}

#[derive(Debug, Default)]
pub struct Endpoint {
    pub id: AssetId,
    pub machine_id: MachineId,
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
            machine_id: value.get("system_id"),
            hostname: value.get("hostname"),
            reg_state: value.get("reg_state"),
        }
    }
}
