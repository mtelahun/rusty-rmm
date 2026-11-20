//! Tunnel connections: each `TunnelOpen` becomes a TCP connection to `127.0.0.1:<port>` whose
//! bytes are relayed to and from the Server. Never dials anything but the loopback.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use rustyrmm_proto::agent::v1::{
    agent_message, AgentMessage, TunnelClose, TunnelData, TunnelOpen, TunnelOpenResult,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// Chunks queued per connection before the Session loop waits (backpressure).
const QUEUE: usize = 32;
const CHUNK: usize = 16 * 1024;

/// One Session's open connections, by the Server's connection id. Dropped with the Session,
/// which ends every connection.
#[derive(Default)]
pub struct Connections {
    to_socket: HashMap<String, mpsc::Sender<Vec<u8>>>,
}

impl Connections {
    pub fn open(&mut self, to_server: &mpsc::Sender<AgentMessage>, open: TunnelOpen) {
        let (tx, rx) = mpsc::channel(QUEUE);
        self.to_socket.insert(open.connection_id.clone(), tx);
        tokio::spawn(relay(to_server.clone(), open.connection_id, open.port, rx));
    }

    /// Waits when the connection's queue is full: the Session loop pauses rather than buffering
    /// without limit. shortcut: a stalled local peer also pauses heartbeats; add per-connection
    /// credit in the protocol if that matters.
    pub async fn data(&mut self, data: TunnelData) {
        let Some(tx) = self.to_socket.get(&data.connection_id) else {
            return;
        };
        if tx.send(data.data).await.is_err() {
            self.to_socket.remove(&data.connection_id);
        }
    }

    pub fn close(&mut self, close: TunnelClose) {
        self.to_socket.remove(&close.connection_id);
    }
}

fn message(m: agent_message::Message) -> AgentMessage {
    AgentMessage { message: Some(m) }
}

async fn relay(
    to_server: mpsc::Sender<AgentMessage>,
    connection_id: String,
    port: u32,
    mut from_server: mpsc::Receiver<Vec<u8>>,
) {
    let connected = match u16::try_from(port) {
        Ok(port) => TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|e| e.to_string()),
        Err(_) => Err(format!("invalid port {port}")),
    };
    let result = TunnelOpenResult {
        connection_id: connection_id.clone(),
        error: connected.as_ref().err().cloned().unwrap_or_default(),
    };
    let sent = to_server
        .send(message(agent_message::Message::TunnelOpenResult(result)))
        .await;
    let Ok(stream) = connected else { return };
    if sent.is_err() {
        return;
    }

    let (mut read, mut write) = stream.into_split();
    let id = connection_id.clone();
    let tx = to_server.clone();
    let reader = async move {
        let mut buf = vec![0; CHUNK];
        loop {
            match read.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let data = TunnelData {
                        connection_id: id.clone(),
                        data: buf[..n].to_vec(),
                    };
                    if tx
                        .send(message(agent_message::Message::TunnelData(data)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    };
    let writer = async {
        while let Some(chunk) = from_server.recv().await {
            if write.write_all(&chunk).await.is_err() {
                return;
            }
        }
    };
    // Either side ending closes the connection.
    tokio::select! { _ = reader => {}, _ = writer => {} }
    let close = TunnelClose { connection_id };
    let _ = to_server
        .send(message(agent_message::Message::TunnelClose(close)))
        .await;
}
