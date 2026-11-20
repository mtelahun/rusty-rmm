//! Tunnels: the in-memory registry of Operator-opened relays, and the routing of Agent messages
//! to them. The Server opens no listening port; the CLI's stream carries the bytes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rustyrmm_proto::agent::v1 as agent;
use rustyrmm_proto::operator::v1::{
    tunnel_client_message, tunnel_server_message, ConnectionClose, ConnectionData,
    TunnelClientMessage, TunnelServerMessage,
};
use tokio::sync::{mpsc, Notify};
use tonic::{Status, Streaming};
use uuid::Uuid;

use crate::sessions::{Outbound, Sessions};

/// The stream to the Operator's CLI.
pub type ToCli = mpsc::Sender<Result<TunnelServerMessage, Status>>;

struct Tunnel {
    owner: Uuid,
    endpoint_id: Uuid,
    /// The Agent Session the Tunnel's connections live on; the Tunnel ends with it.
    session_id: u64,
    port: u16,
    opened_at: DateTime<Utc>,
    to_cli: ToCli,
    close: Arc<Notify>,
}

pub struct TunnelRow {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub port: u16,
    pub opened_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct Tunnels {
    live: Mutex<HashMap<Uuid, Tunnel>>,
}

/// The id the Agent knows a connection by: unique per Endpoint, and it names its Tunnel.
pub fn agent_connection_id(tunnel_id: Uuid, connection_id: u64) -> String {
    format!("{tunnel_id}/{connection_id}")
}

impl Tunnels {
    /// Registers a Tunnel. The returned `Notify` fires when the Tunnel is closed or its Session ends.
    pub fn open(
        &self,
        owner: Uuid,
        endpoint_id: Uuid,
        session_id: u64,
        port: u16,
        to_cli: ToCli,
    ) -> (Uuid, Arc<Notify>) {
        let id = Uuid::now_v7();
        let close = Arc::new(Notify::new());
        self.live.lock().unwrap().insert(
            id,
            Tunnel {
                owner,
                endpoint_id,
                session_id,
                port,
                opened_at: Utc::now(),
                to_cli,
                close: close.clone(),
            },
        );
        (id, close)
    }

    pub fn remove(&self, id: Uuid) {
        self.live.lock().unwrap().remove(&id);
    }

    pub fn list(&self, owner: Uuid) -> Vec<TunnelRow> {
        let live = self.live.lock().unwrap();
        let mut rows: Vec<_> = live
            .iter()
            .filter(|(_, t)| t.owner == owner)
            .map(|(id, t)| TunnelRow {
                id: *id,
                endpoint_id: t.endpoint_id,
                port: t.port,
                opened_at: t.opened_at,
            })
            .collect();
        rows.sort_by_key(|r| r.id);
        rows
    }

    /// Closes the owner's Tunnel; false if there is no such Tunnel of theirs.
    pub fn close(&self, owner: Uuid, id: Uuid) -> bool {
        let mut live = self.live.lock().unwrap();
        if live.get(&id).is_none_or(|t| t.owner != owner) {
            return false;
        }
        live.remove(&id).expect("just found").close.notify_one();
        true
    }

    /// Ends every Tunnel on the Session (it closed, was replaced or Revoked).
    pub fn end_session(&self, endpoint_id: Uuid, session_id: u64) {
        self.live.lock().unwrap().retain(|_, t| {
            let ended = t.endpoint_id == endpoint_id && t.session_id == session_id;
            if ended {
                t.close.notify_one();
            }
            !ended
        });
    }

    /// Bytes from the Endpoint for one of its connections, to the CLI.
    pub async fn data_from_agent(&self, endpoint_id: Uuid, agent_conn: &str, data: Vec<u8>) {
        if let Some((to_cli, connection_id)) = self.route(endpoint_id, agent_conn) {
            let msg = tunnel_server_message::Message::Data(ConnectionData {
                connection_id,
                data,
            });
            let _ = to_cli
                .send(Ok(TunnelServerMessage { message: Some(msg) }))
                .await;
        }
    }

    /// The Endpoint closed a connection, or failed to open it (`error`).
    pub async fn close_from_agent(&self, endpoint_id: Uuid, agent_conn: &str, error: String) {
        if let Some((to_cli, connection_id)) = self.route(endpoint_id, agent_conn) {
            let msg = tunnel_server_message::Message::ConnectionClose(ConnectionClose {
                connection_id,
                error,
            });
            let _ = to_cli
                .send(Ok(TunnelServerMessage { message: Some(msg) }))
                .await;
        }
    }

    /// Finds the CLI stream for an Agent's connection id. The Endpoint must own the Tunnel, so
    /// one Agent cannot write into another's connections.
    fn route(&self, endpoint_id: Uuid, agent_conn: &str) -> Option<(ToCli, u64)> {
        let (tunnel, conn) = agent_conn.split_once('/')?;
        let live = self.live.lock().unwrap();
        let t = live.get(&tunnel.parse::<Uuid>().ok()?)?;
        if t.endpoint_id != endpoint_id {
            return None;
        }
        Some((t.to_cli.clone(), conn.parse().ok()?))
    }
}

/// Relays one Tunnel's CLI stream to the Endpoint's Session until the CLI stream ends or the
/// Tunnel is closed. Always unregisters the Tunnel and closes its connections on the Agent.
pub async fn relay(
    sessions: Arc<Sessions>,
    tunnel_id: Uuid,
    port: u16,
    close: Arc<Notify>,
    to_agent: Outbound,
    mut from_cli: Streaming<TunnelClientMessage>,
) {
    use tunnel_client_message::Message;
    let agent_msg = |m| Ok(agent::ServerMessage { message: Some(m) });
    let close_msg = |connection_id| {
        agent_msg(agent::server_message::Message::TunnelClose(
            agent::TunnelClose {
                connection_id: agent_connection_id(tunnel_id, connection_id),
            },
        ))
    };
    let mut conns = HashSet::new();
    loop {
        let msg = tokio::select! {
            _ = close.notified() => break,
            msg = from_cli.message() => msg,
        };
        let Ok(Some(TunnelClientMessage { message: Some(msg) })) = msg else {
            break;
        };
        let out = match msg {
            Message::ConnectionOpen(o) => {
                conns.insert(o.connection_id);
                agent_msg(agent::server_message::Message::TunnelOpen(
                    agent::TunnelOpen {
                        tunnel_id: tunnel_id.to_string(),
                        connection_id: agent_connection_id(tunnel_id, o.connection_id),
                        port: port.into(),
                    },
                ))
            }
            Message::Data(d) if conns.contains(&d.connection_id) => agent_msg(
                agent::server_message::Message::TunnelData(agent::TunnelData {
                    connection_id: agent_connection_id(tunnel_id, d.connection_id),
                    data: d.data,
                }),
            ),
            Message::ConnectionClose(c) if conns.remove(&c.connection_id) => {
                close_msg(c.connection_id)
            }
            // A second `open`, or data for a connection that is not open.
            _ => continue,
        };
        if to_agent.send(out).await.is_err() {
            break;
        }
    }
    sessions.tunnels.remove(tunnel_id);
    for connection_id in conns {
        let _ = to_agent.try_send(close_msg(connection_id));
    }
}
