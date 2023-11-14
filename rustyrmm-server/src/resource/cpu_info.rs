use rustyrmm_types::ids::{AssetId, CpuId};
use tokio_postgres::Row;

use super::Resource;

#[derive(Debug, Default)]
pub struct CpuInfo {
    pub id: AssetId,
    pub core_count: u32,
    pub thread_count: u32,
}

#[derive(Debug, Default)]
pub struct Cpu {
    pub cpu_info_id: AssetId,
    pub cpu_id: CpuId,
    pub name: String,
    pub vendor_id: String,
    pub brand: String,
    pub frequency: String,
}

impl Resource for CpuInfo {
    const NAME: &'static str = "cpu_info";
}

impl Resource for Cpu {
    const NAME: &'static str = "cpu";
}

impl From<Row> for CpuInfo {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
            core_count: value.get("core_count"),
            thread_count: value.get("thread_count"),
        }
    }
}

impl From<Row> for Cpu {
    fn from(value: Row) -> Self {
        Self {
            cpu_info_id: value.get("id"),
            cpu_id: value.get("cpu_id"),
            name: value.get("name"),
            vendor_id: value.get("vendor_id"),
            brand: value.get("brand"),
            frequency: value.get("frequency"),
        }
    }
}
