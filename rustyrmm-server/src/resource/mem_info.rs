use rustyrmm_types::ids::AssetId;
use tokio_postgres::Row;

use super::Resource;

#[derive(Debug, Default)]
pub struct MemInfo {
    pub id: AssetId,
    pub total: i64,
    pub used: i64,
}

impl Resource for MemInfo {
    const NAME: &'static str = "mem_info";
}

impl From<Row> for MemInfo {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
            total: value.get("total"),
            used: value.get("used"),
        }
    }
}
