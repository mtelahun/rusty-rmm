use std::error::Error;

use rustyrmm_types::ids::{AssetId, MachineId};
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
        let machine_id = MachineId::from(request.system_uuid);
        let endpoint = Endpoint {
            id,
            machine_id,
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
        let _request = request.into_inner();
        todo!()
    }
}
