use std::error::Error;

use crate::{
    db::DBCon,
    resource::{endpoint::Endpoint, Resource},
};

pub async fn create(db_con: DBCon, asset: Endpoint) -> Result<Endpoint, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, system_serial_number, system_sku_number, hostname, reg_state) VALUES ($1, $2, $3, $4, $5::RegistrationState) RETURNING *",
        Endpoint::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &asset.id,
                &asset.system_serial_number,
                &asset.system_sku_number,
                &asset.hostname,
                &asset.reg_state,
            ],
        )
        .await?;

    Ok(row.into())
}
