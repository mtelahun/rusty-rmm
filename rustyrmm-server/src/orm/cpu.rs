use std::error::Error;

use crate::{
    db::DBCon,
    resource::{cpu_info::Cpu, Resource},
};

pub async fn create(db_con: DBCon, cpu: Cpu) -> Result<Cpu, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (cpu_info_id, cpu_id, name, vendor_id, brand, frequency) 
            VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        Cpu::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &cpu.cpu_info_id,
                &cpu.cpu_id,
                &cpu.name,
                &cpu.vendor_id,
                &cpu.brand,
                &cpu.frequency,
            ],
        )
        .await?;

    Ok(row.into())
}
