//! Live Agent Sessions. Online status lives here, in memory: an Endpoint is Online while it has
//! a registered Session. A newer Session for the same Endpoint replaces and closes the old one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustyrmm_proto::agent::v1::ServerMessage;
use tokio::sync::{mpsc, Notify};
use tonic::Status;
use uuid::Uuid;

pub type Outbound = mpsc::Sender<Result<ServerMessage, Status>>;

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
    // Read by Command dispatch in Task: Server Commands and background jobs.
    #[allow(dead_code)]
    outbound: Outbound,
    close: Arc<Notify>,
}

#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<Uuid, Handle>>,
    next_id: AtomicU64,
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
