//! Commands: dispatched over a live Session, run at most once, results stored once.

use std::time::Duration;

use rustyrmm_proto::agent::v1::{server_message, ServerMessage};
use rustyrmm_proto::common::v1::{Command, CommandResult, Interpreter};
use rustyrmm_proto::operator::v1::CommandStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::sessions::{LiveError, Sessions};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error(transparent)]
    Live(#[from] LiveError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct NewCommand {
    pub endpoint_id: Uuid,
    pub operator_username: String,
    pub interpreter: Interpreter,
    pub script: String,
    pub timeout_seconds: u32,
}

/// Records the Command and sends it to the Endpoint's live Session. Commands are never queued:
/// an Offline Endpoint gets `LiveError::Offline` and nothing is stored.
pub async fn dispatch(
    pool: &PgPool,
    sessions: &Sessions,
    cmd: NewCommand,
) -> Result<Uuid, DispatchError> {
    let outbound = sessions.sender(cmd.endpoint_id).ok_or(LiveError::Offline)?;
    let id = Uuid::now_v7();
    // Stored before sending so a fast result always finds its row.
    sqlx::query(
        "INSERT INTO commands (id, endpoint_id, endpoint_hostname, operator_username,
                               interpreter, script, timeout_seconds, status)
         SELECT $1, id, hostname, $2, $3, $4, $5, $6 FROM endpoints WHERE id = $7",
    )
    .bind(id)
    .bind(&cmd.operator_username)
    .bind(cmd.interpreter as i16)
    .bind(&cmd.script)
    .bind(i32::try_from(cmd.timeout_seconds).unwrap_or(i32::MAX))
    .bind(CommandStatus::Dispatched as i16)
    .bind(cmd.endpoint_id)
    .execute(pool)
    .await?;

    let message = ServerMessage {
        message: Some(server_message::Message::Command(Command {
            id: id.to_string(),
            interpreter: cmd.interpreter as i32,
            script: cmd.script,
            timeout_seconds: cmd.timeout_seconds,
        })),
    };
    if outbound.send(Ok(message)).await.is_err() {
        // The Session closed between lookup and send: it never reached the Agent.
        sqlx::query("DELETE FROM commands WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
        return Err(LiveError::Offline.into());
    }
    Ok(id)
}

/// Stores a Command result. Only the Endpoint the Command was sent to can complete it, and only
/// once: a result resent after a reconnect is ignored. A late result still completes a Command
/// already marked "outcome unknown".
pub async fn record_result(
    pool: &PgPool,
    endpoint_id: Uuid,
    result: &CommandResult,
) -> sqlx::Result<()> {
    let Ok(id) = result.id.parse::<Uuid>() else {
        return Ok(()); // Not one of ours.
    };
    sqlx::query(
        "UPDATE commands SET status = $3, exit_code = $4, stdout = $5, stderr = $6,
                truncated = $7, started_at = $8, finished_at = $9, timed_out = $10
         WHERE id = $1 AND endpoint_id = $2 AND status <> $3",
    )
    .bind(id)
    .bind(endpoint_id)
    .bind(CommandStatus::Completed as i16)
    .bind(result.exit_code)
    .bind(&result.stdout)
    .bind(&result.stderr)
    .bind(result.truncated)
    .bind(crate::ingest::agent_time(result.started_at.as_ref()))
    .bind(crate::ingest::agent_time(result.finished_at.as_ref()))
    .bind(result.timed_out)
    .execute(pool)
    .await?;
    Ok(())
}

/// Marks Commands with no result `grace` past their own timeout as "outcome unknown"
/// (e.g. the Agent restarted mid-run). Returns how many were marked.
pub async fn mark_outcome_unknown(pool: &PgPool, grace: Duration) -> sqlx::Result<u64> {
    let done = sqlx::query(
        "UPDATE commands SET status = $1
         WHERE status = $2
           AND created_at + make_interval(secs => timeout_seconds + $3) < now()",
    )
    .bind(CommandStatus::OutcomeUnknown as i16)
    .bind(CommandStatus::Dispatched as i16)
    .bind(grace.as_secs_f64())
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}
