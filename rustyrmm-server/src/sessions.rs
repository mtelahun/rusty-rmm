//! Live Agent Sessions. Online status lives here, in memory: an Endpoint is Online while it has
//! a registered Session. A newer Session for the same Endpoint replaces and closes the old one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustyrmm_proto::agent::v1::{server_message, ProcessList, ProcessListRequest, ServerMessage};
use rustyrmm_proto::common::v1::ProcessInfo;
use tokio::sync::{mpsc, oneshot, Notify};
use tonic::Status;
use uuid::Uuid;

pub type Outbound = mpsc::Sender<Result<ServerMessage, Status>>;
type ProcessListReply = oneshot::Sender<Vec<ProcessInfo>>;

/// Session timing, sent to Agents in HelloAck where relevant.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub heartbeat_interval: Duration,
    /// A Session with no message for this long is closed and its Endpoint goes Offline.
    pub offline_after: Duration,
    pub sample_interval: Duration,
    pub inventory_interval: Duration,
    /// Minimum gap between `last_seen_at` writes for one Endpoint.
    pub last_seen_write_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(30),
            offline_after: Duration::from_secs(90),
            sample_interval: Duration::from_secs(30),
            inventory_interval: Duration::from_secs(3600),
            last_seen_write_interval: Duration::from_secs(60),
        }
    }
}

struct Handle {
    id: u64,
    outbound: Outbound,
    close: Arc<Notify>,
}

#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<Uuid, Handle>>,
    next_id: AtomicU64,
    /// Process-list requests awaiting the Agent's reply, keyed by (Endpoint, request id).
    pending_process_lists: Mutex<HashMap<(Uuid, String), ProcessListReply>>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LiveError {
    #[error("Endpoint is Offline")]
    Offline,
    #[error("Endpoint did not reply in time")]
    Timeout,
}

/// A registered Session: `close` fires when it is replaced or disconnected.
pub struct Registration {
    pub id: u64,
    pub close: Arc<Notify>,
}

impl Sessions {
    /// Registers a Session, replacing (and closing) any existing one for the Endpoint.
    pub fn register(&self, endpoint_id: Uuid, outbound: Outbound) -> Registration {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let close = Arc::new(Notify::new());
        let old = self.live.lock().unwrap().insert(
            endpoint_id,
            Handle {
                id,
                outbound,
                close: close.clone(),
            },
        );
        if let Some(old) = old {
            // notify_one stores a permit, so the old Session sees it even if not yet waiting.
            old.close.notify_one();
        }
        Registration { id, close }
    }

    /// The live Session's outbound channel, if the Endpoint is Online.
    pub fn sender(&self, endpoint_id: Uuid) -> Option<Outbound> {
        let live = self.live.lock().unwrap();
        live.get(&endpoint_id).map(|h| h.outbound.clone())
    }

    /// Asks the Endpoint for its process list and waits up to `wait` for the reply.
    /// Nothing is stored.
    // Called by the Operator API's GetProcessList once it exists.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn request_process_list(
        &self,
        endpoint_id: Uuid,
        wait: Duration,
    ) -> Result<Vec<ProcessInfo>, LiveError> {
        let outbound = self.sender(endpoint_id).ok_or(LiveError::Offline)?;
        let request_id = Uuid::now_v7().to_string();
        let key = (endpoint_id, request_id.clone());
        let (tx, rx) = oneshot::channel();
        self.pending_process_lists
            .lock()
            .unwrap()
            .insert(key.clone(), tx);
        let request = ServerMessage {
            message: Some(server_message::Message::ProcessListRequest(
                ProcessListRequest { request_id },
            )),
        };
        let result = match outbound.send(Ok(request)).await {
            Err(_) => Err(LiveError::Offline),
            Ok(()) => match tokio::time::timeout(wait, rx).await {
                Ok(Ok(processes)) => Ok(processes),
                Ok(Err(_)) | Err(_) => Err(LiveError::Timeout),
            },
        };
        self.pending_process_lists.lock().unwrap().remove(&key);
        result
    }

    /// Delivers an Agent's process list to the request waiting for it, if any.
    /// Keyed by Endpoint too, so one Agent cannot answer another's request.
    pub fn complete_process_list(&self, endpoint_id: Uuid, list: ProcessList) {
        let key = (endpoint_id, list.request_id);
        if let Some(tx) = self.pending_process_lists.lock().unwrap().remove(&key) {
            let _ = tx.send(list.processes);
        }
    }

    /// Removes the Session if it is still the registered one (a replacement may already exist).
    pub fn remove(&self, endpoint_id: Uuid, session_id: u64) {
        let mut live = self.live.lock().unwrap();
        if live.get(&endpoint_id).is_some_and(|h| h.id == session_id) {
            live.remove(&endpoint_id);
        }
    }

    // Used by the Operator API (GetEndpoint, ListEndpoints) once it exists.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_online(&self, endpoint_id: Uuid) -> bool {
        self.live.lock().unwrap().contains_key(&endpoint_id)
    }

    /// Closes the Endpoint's Session, e.g. when it is Revoked or deleted.
    // Called by the Operator API's RevokeEndpoint/DeleteEndpoint once it exists.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn disconnect(&self, endpoint_id: Uuid) {
        if let Some(old) = self.live.lock().unwrap().remove(&endpoint_id) {
            old.close.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn newest_session_wins_and_old_removal_is_ignored() {
        let sessions = Sessions::default();
        let ep = Uuid::now_v7();
        let (tx1, _rx1) = mpsc::channel(1);
        let (tx2, _rx2) = mpsc::channel(1);
        let first = sessions.register(ep, tx1);
        let second = sessions.register(ep, tx2);

        // The first Session is told to close...
        tokio::time::timeout(Duration::from_secs(1), first.close.notified())
            .await
            .expect("old Session should be closed");
        // ...and its cleanup must not unregister the replacement.
        sessions.remove(ep, first.id);
        assert!(sessions.is_online(ep));

        sessions.remove(ep, second.id);
        assert!(!sessions.is_online(ep));
    }

    #[tokio::test]
    async fn disconnect_closes_and_takes_offline() {
        let sessions = Sessions::default();
        let ep = Uuid::now_v7();
        let (tx, _rx) = mpsc::channel(1);
        let reg = sessions.register(ep, tx);
        sessions.disconnect(ep);
        assert!(!sessions.is_online(ep));
        tokio::time::timeout(Duration::from_secs(1), reg.close.notified())
            .await
            .expect("Session should be closed");
    }
}
