use std::error::Error;

use crate::{
    db::DBCon,
    resource::{endpoint::Endpoint, Resource},
};

pub async fn create(db_con: DBCon, asset: Endpoint) -> Result<Endpoint, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, system_id, hostname, reg_state) VALUES ($1, $2, $3, $4) RETURNING *",
        Endpoint::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &asset.id,
                &asset.machine_id,
                &asset.hostname,
                &asset.reg_state,
            ],
        )
        .await?;

    Ok(row.into())
}
