use std::error::Error;

use crate::{
    db::DBCon,
    resource::{endpoint_detail::EndpointDetail, Resource},
};

pub async fn create(
    db_con: DBCon,
    detail: EndpointDetail,
) -> Result<EndpointDetail, Box<dyn Error>> {
    let statement = format!(
        "INSERT INTO {} (id, system_id, hostname, os, cpu, disk, mem, net, client_ver)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING *",
        EndpointDetail::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &detail.id,
                &detail.machine_id,
                &detail.hostname,
                &detail.os,
                &detail.cpu,
                &detail.disk,
                &detail.mem,
                &detail.net,
                &detail.client_ver,
            ],
        )
        .await?;

    Ok(row.into())
}
