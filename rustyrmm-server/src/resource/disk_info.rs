use postgres_types::{FromSql, ToSql};
use rust_decimal::Decimal;
use rustyrmm_proto::endpoint_registration::DiskType;
use rustyrmm_types::ids::{AssetId, DiskId};
use tokio_postgres::Row;

use super::Resource;

#[derive(Debug, Default, ToSql, FromSql)]
#[postgres(name = "DiskType")]
pub enum DiskTypeDb {
    #[default]
    #[postgres(name = "unknown")]
    Unknown,
    #[postgres(name = "hdd")]
    Hdd,
    #[postgres(name = "ssd")]
    Ssd,
}

#[derive(Debug, Default)]
pub struct DiskInfo {
    pub id: AssetId,
}

#[derive(Debug, Default)]
pub struct Disk {
    pub disk_info_id: AssetId,
    pub disk_id: DiskId,
    pub disk_name: String,
    pub disk_type: DiskTypeDb,
    pub filesystem: String,
    pub mount_point: String,
    pub disk_size: Decimal,
    pub disk_free: Decimal,
}

impl From<DiskType> for DiskTypeDb {
    fn from(value: DiskType) -> Self {
        match value {
            DiskType::TypeUnknown => Self::Unknown,
            DiskType::TypeHdd => Self::Hdd,
            DiskType::TypeSsd => Self::Ssd,
        }
    }
}

impl Resource for DiskInfo {
    const NAME: &'static str = "disk_info";
}

impl Resource for Disk {
    const NAME: &'static str = "disk";
}

impl From<Row> for DiskInfo {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
        }
    }
}

impl From<Row> for Disk {
    fn from(value: Row) -> Self {
        Self {
            disk_info_id: value.get("id"),
            disk_id: value.get("disk_id"),
            disk_name: value.get("disk_name"),
            disk_type: value.get("disk_type"),
            filesystem: value.get("filesystem"),
            mount_point: value.get("mount_point"),
            disk_size: value.get("disk_size"),
            disk_free: value.get("disk_free"),
        }
    }
}
