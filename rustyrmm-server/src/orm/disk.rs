use std::error::Error;

use crate::{
    db::DBCon,
    resource::{disk_info::Disk, Resource},
};

pub async fn create(db_con: DBCon, disk: Disk) -> Result<Disk, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (disk_info_id, disk_id, disk_name, disk_type, filesystem, mount_point, disk_size, disk_free) 
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *",
        Disk::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &disk.disk_info_id,
                &disk.disk_id,
                &disk.disk_name,
                &disk.disk_type,
                &disk.filesystem,
                &disk.mount_point,
                &disk.disk_size,
                &disk.disk_free,
            ],
        )
        .await?;

    Ok(row.into())
}
