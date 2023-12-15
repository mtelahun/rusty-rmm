use std::error::Error;

use crate::{
    db::DBCon,
    resource::{net_info::NetIf, Resource},
};

pub async fn create(db_con: DBCon, net_if: NetIf) -> Result<NetIf, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (net_info_id, if_id, name, mac, ip4, ip6) 
            VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        NetIf::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &net_if.net_info_id,
                &net_if.if_id,
                &net_if.name,
                &net_if.mac,
                &net_if.ip4,
                &net_if.ip6,
            ],
        )
        .await?;

    Ok(row.into())
}
