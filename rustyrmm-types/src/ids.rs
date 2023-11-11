use std::{error::Error, ops::Deref};

use postgres_types::{FromSql, ToSql};
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "assetid")]
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

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "systemuuid")]
pub struct SystemUuid(uuid::Uuid);

impl SystemUuid {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn try_parse(value: &str) -> Result<SystemUuid, Box<dyn Error>> {
        Ok(Self(Uuid::try_parse(value)?))
    }
}

impl std::fmt::Display for SystemUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for SystemUuid {
    type Target = uuid::Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Default, ToSql, FromSql)]
#[postgres(name = "machineid")]
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
