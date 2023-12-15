use std::error::Error;

use crate::{
    db::DBCon,
    resource::{net_info::NetInfo, Resource},
};

pub async fn create(db_con: DBCon, net: NetInfo) -> Result<NetInfo, Box<dyn Error>> {
    let statement = format!("INSERT INTO {} (id) VALUES ($1) RETURNING *", NetInfo::NAME,);

    let row = db_con.query_one(&statement, &[&net.id]).await?;

    Ok(row.into())
}
