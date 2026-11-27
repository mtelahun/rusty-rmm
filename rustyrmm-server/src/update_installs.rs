//! Update Installs: sent over a live Session, run by the Agent apart from itself, outcome
//! stored once and acknowledged on every receipt.

use prost::Message;
use rustyrmm_proto::agent::v1::{server_message, ServerMessage, UpdateInstallAck};
use rustyrmm_proto::common::v1::{
    Inventory, RebootPolicy, UpdateInstallOutcome, UpdateInstallOutcomeKind, UpdateInstallRequest,
    UpdateResultStatus, UpdateSelection,
};
use rustyrmm_proto::operator::v1::UpdateInstallStatus;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::sessions::Sessions;

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("no such Endpoint")]
    NotFound,
    #[error("Update Install is only supported on Linux Endpoints (apt, dnf)")]
    Unsupported,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct NewUpdateInstall {
    pub endpoint_id: Uuid,
    pub operator_username: String,
    pub selection: UpdateSelection,
}

/// Records the Update Install and sends it to the Endpoint's live Session. Never queued: an
/// Offline Endpoint's record ends Skipped (Offline), and a second unfinished one for the same
/// Endpoint ends Rejected (busy); neither is sent. Returns the record's id and status.
pub async fn request(
    pool: &PgPool,
    sessions: &Sessions,
    new: NewUpdateInstall,
) -> Result<(Uuid, UpdateInstallStatus), RequestError> {
    let inventory: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT (SELECT inventory FROM inventories WHERE endpoint_id = e.id
                 ORDER BY received_at DESC, id DESC LIMIT 1)
         FROM endpoints e WHERE e.id = $1",
    )
    .bind(new.endpoint_id)
    .fetch_optional(pool)
    .await?
    .ok_or(RequestError::NotFound)?;
    // An Endpoint that has not reported its OS yet is not known to be Linux.
    let linux = inventory
        .and_then(|b| Inventory::decode(b.as_slice()).ok())
        .and_then(|i| i.os)
        .is_some_and(|os| os.family == "linux");
    if !linux {
        return Err(RequestError::Unsupported);
    }

    let id = Uuid::now_v7();
    let Some(outbound) = sessions.sender(new.endpoint_id) else {
        insert(pool, id, &new, UpdateInstallStatus::SkippedOffline).await?;
        return Ok((id, UpdateInstallStatus::SkippedOffline));
    };
    // The partial unique index decides "busy", so two concurrent requests cannot both pass.
    // Stored before sending so a fast "started" or outcome always finds its row.
    match insert(pool, id, &new, UpdateInstallStatus::Requested).await {
        Ok(()) => {}
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            insert(pool, id, &new, UpdateInstallStatus::RejectedBusy).await?;
            return Ok((id, UpdateInstallStatus::RejectedBusy));
        }
        Err(e) => return Err(e.into()),
    }

    let message = ServerMessage {
        message: Some(server_message::Message::UpdateInstall(
            UpdateInstallRequest {
                id: id.to_string(),
                selection: new.selection as i32,
                reboot: RebootPolicy::Never as i32,
            },
        )),
    };
    if outbound.send(Ok(message)).await.is_err() {
        // The Session closed between lookup and send: it never reached the Agent.
        set_status(pool, id, UpdateInstallStatus::SkippedOffline).await?;
        return Ok((id, UpdateInstallStatus::SkippedOffline));
    }
    Ok((id, UpdateInstallStatus::Requested))
}

async fn insert(
    pool: &PgPool,
    id: Uuid,
    new: &NewUpdateInstall,
    status: UpdateInstallStatus,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO update_installs (id, endpoint_id, endpoint_hostname, operator_username,
                                     selection, reboot, status)
         SELECT $1, id, hostname, $2, $3, $4, $5 FROM endpoints WHERE id = $6",
    )
    .bind(id)
    .bind(&new.operator_username)
    .bind(new.selection as i16)
    .bind(RebootPolicy::Never as i16)
    .bind(status as i16)
    .bind(new.endpoint_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn set_status(pool: &PgPool, id: Uuid, status: UpdateInstallStatus) -> sqlx::Result<()> {
    sqlx::query("UPDATE update_installs SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(status as i16)
        .execute(pool)
        .await?;
    Ok(())
}

/// The Agent started the install: Requested becomes Running. Only the Endpoint it was sent to
/// can start it.
pub async fn record_started(pool: &PgPool, endpoint_id: Uuid, id: &str) -> sqlx::Result<()> {
    let Ok(id) = id.parse::<Uuid>() else {
        return Ok(()); // Not one of ours.
    };
    sqlx::query(
        "UPDATE update_installs SET status = $3, started_at = now()
         WHERE id = $1 AND endpoint_id = $2 AND status = $4",
    )
    .bind(id)
    .bind(endpoint_id)
    .bind(UpdateInstallStatus::Running as i16)
    .bind(UpdateInstallStatus::Requested as i16)
    .execute(pool)
    .await?;
    Ok(())
}

/// The record status an Agent outcome ends in; None for an outcome this Server doesn't know.
pub fn final_status(outcome: &UpdateInstallOutcome) -> Option<UpdateInstallStatus> {
    use UpdateInstallOutcomeKind as Kind;
    Some(match Kind::try_from(outcome.kind).ok()? {
        Kind::RejectedBusy => UpdateInstallStatus::RejectedBusy,
        Kind::Interrupted => UpdateInstallStatus::Interrupted,
        Kind::FailedToStart => UpdateInstallStatus::Failed,
        Kind::Finished => {
            let ok = outcome.results.iter().all(|r| {
                matches!(
                    UpdateResultStatus::try_from(r.status),
                    Ok(UpdateResultStatus::Installed | UpdateResultStatus::NotPending)
                )
            });
            if ok {
                UpdateInstallStatus::Succeeded
            } else {
                UpdateInstallStatus::Failed
            }
        }
        Kind::Unspecified => return None,
    })
}

/// Stores an Update Install outcome. Only the Endpoint it was sent to can end it, and only once:
/// a resent outcome is ignored. A late outcome with results still ends one marked Interrupted.
pub async fn record_outcome(
    pool: &PgPool,
    endpoint_id: Uuid,
    outcome: &UpdateInstallOutcome,
) -> sqlx::Result<()> {
    let (Ok(id), Some(status)) = (outcome.id.parse::<Uuid>(), final_status(outcome)) else {
        return Ok(()); // Not one of ours.
    };
    let late_ok = matches!(
        status,
        UpdateInstallStatus::Succeeded | UpdateInstallStatus::Failed
    );
    let results: Vec<_> = outcome
        .results
        .iter()
        .map(|r| json!({"package": r.package, "status": r.status, "exit_code": r.exit_code}))
        .collect();
    let updates = outcome.updates.unwrap_or_default();
    sqlx::query(
        "UPDATE update_installs
         SET status = $3, results = $4, updates_regular = $5, updates_security = $6,
             started_at = COALESCE(started_at, $7), finished_at = $8
         WHERE id = $1 AND endpoint_id = $2
           AND (status IN ($9, $10) OR (status = $11 AND $12))",
    )
    .bind(id)
    .bind(endpoint_id)
    .bind(status as i16)
    .bind(serde_json::Value::Array(results))
    .bind(
        updates
            .regular
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
    )
    .bind(
        updates
            .security
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
    )
    .bind(crate::ingest::agent_time(outcome.started_at.as_ref()))
    .bind(crate::ingest::agent_time(outcome.finished_at.as_ref()))
    .bind(UpdateInstallStatus::Requested as i16)
    .bind(UpdateInstallStatus::Running as i16)
    .bind(UpdateInstallStatus::Interrupted as i16)
    .bind(late_ok)
    .execute(pool)
    .await?;
    Ok(())
}

pub fn ack(id: String) -> ServerMessage {
    ServerMessage {
        message: Some(server_message::Message::UpdateInstallAck(
            UpdateInstallAck { id },
        )),
    }
}
