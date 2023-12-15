use std::error::Error;

use crate::{
    db::DBCon,
    resource::{os_info::OsInfo, Resource},
};

pub async fn create(db_con: DBCon, os: OsInfo) -> Result<OsInfo, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, os_name, os_ver, kernel_ver, virt_system, virt_role, tz, machine_id) 
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *;",
        OsInfo::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &os.id,
                &os.os_name,
                &os.os_ver,
                &os.kernel_ver,
                &os.virt_system,
                &os.virt_role,
                &os.tz,
                &os.machine_id,
            ],
        )
        .await?;

    Ok(row.into())
}
