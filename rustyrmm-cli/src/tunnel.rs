//! `rustyrmm tunnel open`: a local listener whose connections are relayed through the Server to
//! a port on an Endpoint's loopback. The Tunnel lasts as long as this process.

use std::collections::HashMap;

use rustyrmm_proto::operator::v1::{
    tunnel_client_message as client, tunnel_server_message as server, ConnectionClose,
    ConnectionData, ConnectionOpen, OpenTunnelRequest, TunnelClientMessage,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::output::details;
use crate::session::Api;

/// Chunks queued per connection before reading from the Server waits (backpressure).
const QUEUE: usize = 32;
const CHUNK: usize = 16 * 1024;

fn message(m: client::Message) -> TunnelClientMessage {
    TunnelClientMessage { message: Some(m) }
}

/// Opens the Tunnel and serves it until Ctrl-C or the stream ends.
pub async fn open(
    api: &mut Api,
    endpoint_id: String,
    port: u32,
    bind: &str,
    listen_port: u16,
    json: bool,
) -> anyhow::Result<()> {
    let (to_server, rx) = mpsc::channel(QUEUE);
    to_server
        .send(message(client::Message::Open(OpenTunnelRequest {
            endpoint_id,
            port,
        })))
        .await?;
    let mut from_server = api
        .client
        .open_tunnel(api.req(ReceiverStream::new(rx)))
        .await
        .map_err(crate::status)?
        .into_inner();
    let tunnel_id = match from_server.message().await.map_err(crate::status)? {
        Some(m) => match m.message {
            Some(server::Message::Opened(o)) => o.tunnel_id,
            _ => anyhow::bail!("the Server did not confirm the Tunnel"),
        },
        None => anyhow::bail!("the Server closed the Tunnel"),
    };
    let listener = TcpListener::bind((bind, listen_port)).await?;
    let listening = listener.local_addr()?.to_string();
    println!(
        "{}",
        details(
            &[("tunnel id", tunnel_id), ("listening on", listening)],
            json
        )
    );

    let mut conns: HashMap<u64, mpsc::Sender<Vec<u8>>> = HashMap::new();
    let mut next_id = 0;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                next_id += 1;
                let (tx, rx) = mpsc::channel(QUEUE);
                conns.insert(next_id, tx);
                tokio::spawn(relay(next_id, stream, to_server.clone(), rx));
            }
            msg = from_server.message() => {
                let Some(msg) = msg.map_err(crate::status)? else {
                    eprintln!("Tunnel closed by the Server");
                    return Ok(());
                };
                match msg.message {
                    Some(server::Message::Data(d)) => {
                        // Waits when the connection's queue is full, so nothing grows unbounded.
                        if let Some(tx) = conns.get(&d.connection_id) {
                            if tx.send(d.data).await.is_err() {
                                conns.remove(&d.connection_id);
                            }
                        }
                    }
                    Some(server::Message::ConnectionClose(c)) => {
                        conns.remove(&c.connection_id);
                        if !c.error.is_empty() {
                            eprintln!("connection {}: {}", c.connection_id, c.error);
                        }
                    }
                    Some(server::Message::Opened(_)) | None => {}
                }
            }
        }
    }
}

/// One local connection: its bytes go to the Server, and the Server's bytes come back.
async fn relay(
    id: u64,
    stream: TcpStream,
    to_server: mpsc::Sender<TunnelClientMessage>,
    mut from_server: mpsc::Receiver<Vec<u8>>,
) {
    let opened = ConnectionOpen { connection_id: id };
    if to_server
        .send(message(client::Message::ConnectionOpen(opened)))
        .await
        .is_err()
    {
        return;
    }
    let (mut read, mut write) = stream.into_split();
    let tx = to_server.clone();
    let reader = async move {
        let mut buf = vec![0; CHUNK];
        loop {
            match read.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let data = ConnectionData {
                        connection_id: id,
                        data: buf[..n].to_vec(),
                    };
                    if tx.send(message(client::Message::Data(data))).await.is_err() {
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
    let close = ConnectionClose {
        connection_id: id,
        error: String::new(),
    };
    let _ = to_server
        .send(message(client::Message::ConnectionClose(close)))
        .await;
}
