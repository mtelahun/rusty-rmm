use std::error::Error;

use crate::{
    db::DBCon,
    resource::{mem_info::MemInfo, Resource},
};

pub async fn create(db_con: DBCon, mem: MemInfo) -> Result<MemInfo, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, total, used) VALUES ($1, $2, $3) RETURNING *",
        MemInfo::NAME,
    );

    let row = db_con
        .query_one(&statement, &[&mem.id, &mem.total, &mem.used])
        .await?;

    Ok(row.into())
}
