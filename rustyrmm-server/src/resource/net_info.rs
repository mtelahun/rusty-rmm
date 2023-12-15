use rustyrmm_types::ids::{AssetId, IfId};
use tokio_postgres::Row;

use super::Resource;

#[derive(Debug, Default)]
pub struct NetInfo {
    pub id: AssetId,
}

#[derive(Debug, Default)]
pub struct NetIf {
    pub net_info_id: AssetId,
    pub if_id: IfId,
    pub name: Option<String>,
    pub mac: Option<String>,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
}

impl Resource for NetInfo {
    const NAME: &'static str = "net_info";
}

impl From<Row> for NetInfo {
    fn from(value: Row) -> Self {
        Self {
            id: value.get("id"),
        }
    }
}

impl Resource for NetIf {
    const NAME: &'static str = "net_if";
}

impl From<Row> for NetIf {
    fn from(value: Row) -> Self {
        Self {
            net_info_id: value.get("net_info_id"),
            if_id: value.get("id"),
            name: value.get("name"),
            mac: value.get("mac"),
            ip4: value.get("ip4"),
            ip6: value.get("ip6"),
        }
    }
}
