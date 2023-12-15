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
        "INSERT INTO {} (id, hostname, system_serial_number, system_sku_number, client_ver, os_info_id, cpu_info_id, disk_info_id, mem_info_id, net_info_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING *",
        EndpointDetail::NAME,
    );

    let row = db_con
        .query_one(
            &statement,
            &[
                &detail.id,
                &detail.hostname,
                &detail.system_serial_number,
                &detail.system_sku_number,
                &detail.client_ver,
                &detail.os,
                &detail.cpu,
                &detail.disk,
                &detail.mem,
                &detail.net,
            ],
        )
        .await?;

    Ok(row.into())
}
