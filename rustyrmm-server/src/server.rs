use std::error::Error;

use rustyrmm_types::ids::AssetId;
use rustyrmm_types::registration_state::RegistrationState;
use tonic::{Request, Response, Status};

use rustyrmm_proto::endpoint_registration::registration_service_server::RegistrationService;
use rustyrmm_proto::endpoint_registration::{
    EndpointRegistration, EndpointRegistrationResponse, EndpointUpdate, EndpointUpdateResponse,
    ResponseStatus, RustyRmmId,
};

use crate::db::{DBCon, DBPool};
use crate::orm;
use crate::resource::endpoint::Endpoint;

pub struct EndPt {
    db_pool: DBPool,
}

impl EndPt {
    pub fn new(db_pool: DBPool) -> EndPt {
        Self { db_pool }
    }

    async fn db_con(&self) -> Result<DBCon, Box<dyn Error>> {
        Ok(self.db_pool.get().await?)
    }
}

#[tonic::async_trait]
impl RegistrationService for EndPt {
    async fn register_endpoint(
        &self,
        request: Request<EndpointRegistration>,
    ) -> Result<Response<EndpointRegistrationResponse>, Status> {
        let request = request.into_inner();
        let id: AssetId = AssetId::new();
        let endpoint = Endpoint {
            id,
            system_serial_number: request.system_serial_number,
            system_sku_number: request.system_sku_number,
            hostname: request.hostname,
            reg_state: RegistrationState::New,
        };
        let db_con = self
            .db_con()
            .await
            .map_err(|_| Status::internal("Database error"))?;
        let endpoint = orm::endpoint::create(db_con, endpoint)
            .await
            .map_err(|e| Status::internal(format!("Database Error: {e}")))?;

        let response = EndpointRegistrationResponse {
            status: ResponseStatus::StatusOk.into(),
            id: Some(RustyRmmId {
                uuid: endpoint.id.to_string(),
            }),
        };
        Ok(Response::new(response))
    }

    async fn update_endpoint(
        &self,
        request: Request<EndpointUpdate>,
    ) -> Result<Response<EndpointUpdateResponse>, Status> {
        let request = request.into_inner();
        let rusty_id = match request.id {
            Some(id) => id,
            None => return Err(Status::internal("Endpoint ID missing from request")),
        };
        let mut updated_count = 0;

        let os_info = match request.os {
            Some(os) => Some(crate::resource::os_info::OsInfo {
                id: AssetId::from(rusty_id.clone()),
                os_name: os.full_name,
                os_ver: os.version,
                kernel_ver: os.kernel_version,
                virt_system: os.virt_system,
                virt_role: os.virt_role,
                tz: os.tz,
                machine_id: os.machine_id,
            }),
            None => None,
        };
        let db_con = self
            .db_con()
            .await
            .map_err(|e| Status::internal(format!("database error: {e}")))?;
        if let Some(os) = os_info {
            orm::os_info::create(db_con, os)
                .await
                .map_err(|e| Status::internal(format!("database error: {e}")))?;
            updated_count += 1;
        }

        let response = EndpointUpdateResponse {
            status: ResponseStatus::StatusOk.into(),
            id: Some(rusty_id),
            updated: updated_count,
        };
        Ok(Response::new(response))
    }
}
