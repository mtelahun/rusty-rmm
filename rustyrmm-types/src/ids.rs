use std::{error::Error, ops::Deref};

use postgres_types::{FromSql, ToSql};
use rustyrmm_proto::endpoint_registration::RustyRmmId;
use uuid::Uuid;

#[derive(
    sqlx::Type, Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql,
)]
#[postgres(name = "assetid")]
#[sqlx(type_name = "AssetId")]
pub struct AssetId(uuid::Uuid);

impl AssetId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn try_parse(value: &str) -> Result<AssetId, Box<dyn Error>> {
        Ok(Self(Uuid::try_parse(value)?))
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for AssetId {
    type Target = uuid::Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<RustyRmmId> for AssetId {
    fn from(value: RustyRmmId) -> Self {
        Self(Uuid::parse_str(value.uuid.as_str()).unwrap_or_default())
    }
}

#[derive(sqlx::Type, Debug, Default, ToSql, FromSql)]
#[sqlx(type_name = "MachineId")]
#[postgres(transparent)]
pub struct MachineId(String);

impl std::fmt::Display for MachineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for MachineId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for MachineId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "cpuid")]
pub struct CpuId(i32);

impl CpuId {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Display for CpuId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for CpuId {
    type Target = i32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<i32> for CpuId {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "diskid")]
pub struct DiskId(i32);

impl DiskId {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Display for DiskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for DiskId {
    type Target = i32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<i32> for DiskId {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "ifid")]
pub struct IfId(i32);

impl IfId {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Display for IfId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for IfId {
    type Target = i32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<i32> for IfId {
    fn from(value: i32) -> Self {
        Self(value)
    }
}
