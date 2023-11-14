use std::error::Error;

use crate::{
    db::DBCon,
    resource::{disk_info::DiskInfo, Resource},
};

pub async fn create(db_con: DBCon, cpu: DiskInfo) -> Result<DiskInfo, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id) VALUES ($1) RETURNING *",
        DiskInfo::NAME,
    );

    let row = db_con.query_one(&statement, &[&cpu.id]).await?;

    Ok(row.into())
}
