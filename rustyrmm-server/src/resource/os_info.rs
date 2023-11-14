use rustyrmm_types::ids::AssetId;
use tokio_postgres::Row;

use super::Resource;

#[derive(Debug, Default)]
pub struct OsInfo {
    pub id: AssetId,
    pub name: String,
    pub os_ver: String,
    pub kernel_ver: String,
    pub virt_system: String,
    pub virt_role: String,
    pub tz: String,
}

impl Resource for OsInfo {
    const NAME: &'static str = "os_info";
}

impl From<Row> for OsInfo {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
            name: value.get("os_name"),
            os_ver: value.get("os_ver"),
            kernel_ver: value.get("kernel_ver"),
            virt_system: value.get("virt_system"),
            virt_role: value.get("virt_role"),
            tz: value.get("tz"),
        }
    }
}
