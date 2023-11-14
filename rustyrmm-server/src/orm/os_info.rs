use std::error::Error;

use crate::{
    db::DBCon,
    resource::{endpoint::Endpoint, os_info::OsInfo, Resource},
};

pub async fn create(db_con: DBCon, os: OsInfo) -> Result<Endpoint, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, name, os_ver, kernel_ver, virt_system, virt_role, tz) 
            VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
        OsInfo::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &os.id,
                &os.name,
                &os.os_ver,
                &os.kernel_ver,
                &os.virt_system,
                &os.virt_role,
                &os.tz,
            ],
        )
        .await?;

    Ok(row.into())
}
