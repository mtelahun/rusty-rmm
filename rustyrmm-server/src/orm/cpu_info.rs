use std::error::Error;

use crate::{
    db::DBCon,
    resource::{cpu_info::CpuInfo, Resource},
};

pub async fn create(db_con: DBCon, cpu: CpuInfo) -> Result<CpuInfo, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id) VALUES ($1, $2, $3) RETURNING *",
        CpuInfo::NAME,
    );

    let row = db_con
        .query_one(&statement, &[&cpu.id, &cpu.core_count, &cpu.thread_count])
        .await?;

    Ok(row.into())
}
