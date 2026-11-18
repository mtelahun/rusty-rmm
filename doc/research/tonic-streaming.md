# Research: tonic/prost versions and Server-to-Agent command streaming

Resolves issue #3. Checked 2026-11-18. Terms per CONTEXT.md (Server, Agent, Endpoint).

## 1. Current versions

Source: crates.io API (`max_stable_version`).

| Crate | Repo pin | Current stable |
|---|---|---|
| tonic | 0.10.2 | 0.14.9 |
| tonic-build | 0.10.2 | 0.14.9 (still published) |
| tonic-prost | n/a | 0.14.9 |
| tonic-prost-build | n/a | 0.14.9 |
| tonic-web | n/a | 0.14.9 |
| prost / prost-build | (transitive) | 0.14.6 |

The repo pins tonic 0.10.2 in `rustyrmm-agent` and `rustyrmm-server`, and tonic-build 0.10.2 in `rustyrmm-proto`.

## 2. What changed since 0.10

Sources: tonic `CHANGELOG.md` (frozen at 0.12.3; later versions are in GitHub Releases) and release notes for v0.13.0 and v0.14.0 at https://github.com/hyperium/tonic/releases.

- 0.11: `NamedService` moved to `tonic::server::NamedService`; MSRV 1.70; zstd compression; rustls 0.22.
- 0.12: moved to the hyper 1.0 ecosystem and prost 0.13. Breaking. Custom codecs in build. Channel feature made additive. TLS roots are no longer configured implicitly (opt in via the TLS roots features).
- 0.13: breaking release (axum and other crates bumped). Adds a `Body` type, deflate compression, a UDS transport, `Server::serve`, and `Endpoint::local_address`. Fixes an infinite loop in `tonic::Streaming` on response error (#2199). The `tonic_web::enable` API was removed (use `GrpcWebLayer`).
- 0.14: prost 0.14. The prost integration was extracted into new crates `tonic-prost` and `tonic-prost-build`; the old types are re-exported from there. Adds TCP_KEEPINTVL and TCP_KEEPCNT settings, TLS handshake timeout, load-shed layer, and `Streaming` is `Sync` again. The maintainers say this is hopefully the last major breaking release before the `grpc-rust` effort; new core features unrelated to that are not accepted.

Migration impact for this repo (inferred from the 0.14 release notes, not tested):
- `rustyrmm-proto` build script: `tonic-build` for prost codegen becomes `tonic-prost-build`.
- Generated code needs `tonic-prost` as a runtime dependency (the codec lives there).
- The 0.10 to 0.14 jump crosses hyper 1.0, so any direct hyper/http/axum/tower deps must be aligned.

## 3. Idiomatic pattern: Server pushes commands over an Agent-initiated stream

The Agent is behind NAT or a firewall, so the Agent must dial out. The Server cannot open a connection to the Agent. Define one RPC on the Server where the Agent opens the stream:

```proto
service AgentService {
  // Agent calls this once per connection. Server-to-Agent frames are commands;
  // Agent-to-Server frames are hello/heartbeat/results.
  rpc Connect(stream AgentMessage) returns (stream ServerMessage);
}
```

A bidirectional stream is the right fit when the Agent must also send results and heartbeats on the same channel. If the Agent only needs to receive commands, a server-streaming RPC (`rpc Subscribe(Hello) returns (stream Command)`) is simpler, and the Agent reports results via separate unary RPCs.

Server side (tonic's official bidi example, `examples/src/streaming/server.rs`):
- The handler receives `Request<Streaming<T>>` and returns a boxed `Stream<Item = Result<U, Status>>` as the response type.
- It makes an `mpsc::channel(N)`, spawns a task that reads the inbound stream, and returns `ReceiverStream::new(rx)` as the response. The example comments say the spawn is needed so that connection errors propagate.
- For command push, keep the `mpsc::Sender` in a registry (for example `HashMap<AgentId, Sender<ServerMessage>>`) so any part of the Server can queue a command for a connected Agent. Remove the entry when the inbound stream ends or errors. (Registry design is my inference; the examples do not show one.)
- Disconnect shows up as `Err(Status)` from the inbound stream, or the outbound `tx.send` failing because the receiver was dropped. The example downcasts to `io::Error` / `h2::Error` to detect `BrokenPipe`.

Backpressure: the bounded `mpsc::channel(N)` is the mechanism. `tx.send().await` suspends when the Agent is slow, and `try_send` lets you fail fast and return "Agent busy". HTTP/2 stream and connection flow-control windows (default stream window 65,535 bytes per the `Server`/`Endpoint` docs; `http2_adaptive_window` is available) provide transport-level backpressure underneath. Unbounded channels remove that protection and are not recommended.

Keepalive (docs.rs `tonic::transport::Server` and `Endpoint`):
- Server: `http2_keepalive_interval(Option<Duration>)` sends HTTP/2 PING frames (off unless set); `http2_keepalive_timeout` closes the connection if the ack is late (default 20s); `tcp_keepalive` is also available (ignored with `serve_with_incoming`).
- Agent (`Endpoint`): `http2_keep_alive_interval`, `keep_alive_timeout`, `keep_alive_while_idle(true)`, `tcp_keepalive`.
- gRPC's keepalive guide (https://github.com/grpc/grpc/blob/master/doc/keepalive.md) warns that if a client pings more often than the server permits (server default minimum 5 minutes without data, max 2 strikes), the server sends GOAWAY "too_many_pings". That document is about gRPC core; I did not verify that tonic/hyper's server enforces ping strikes, so test interval choices against the real Server, and put an application-level heartbeat message in the protocol as the authoritative liveness signal.

Reconnect:
- `Channel` is built on a `Reconnect` tower service (`tonic/src/transport/channel/service/connection.rs` uses `Reconnect::new`), so a dropped TCP/HTTP2 connection is re-established for subsequent calls, and `Endpoint::connect_lazy` defers the first connect.
- An in-flight stream is not resumed. The Agent must loop: open `Connect`, run until the stream ends or errors, back off (exponential with jitter), and call `Connect` again. Tonic ships no backoff helper; this loop is application code.
- The Server must treat a reconnect as a new session for the same Agent identity and replace the old registry entry. Commands queued while the Agent was offline need an application-level store and ack protocol; the stream gives no delivery guarantee across reconnects. (Design inference.)

## 4. Browser web UI: tonic-web or REST/JSON gateway?

- `tonic-web` lets a tonic Server accept grpc-web requests directly with no proxy: `Server::builder().accept_http1(true).layer(GrpcWebLayer::new())` (tonic-web README and crate docs). CORS is configured by composing with a cors layer. With TLS, `accept_http1` is not needed because the browser negotiates h2 via ALPN.
- Limit: the grpc-web protocol supports only unary and server-streaming RPCs. Client-streaming and bidirectional streaming are not supported, and server-streaming works only in `grpcwebtext` mode in the reference JS client (https://github.com/grpc/grpc-web README). So the Agent `Connect` bidi stream cannot be consumed from a browser, which is fine since only Agents use it.
- Conclusion: a browser UI can call a Server's unary and server-streaming Operator-facing RPCs directly via tonic-web, but needs a JS grpc-web client and generated stubs. I did not find a first-party source saying REST/JSON is "the norm"; that part of the question is a design choice, not a documented fact. Options: (a) tonic-web for Operator RPCs, or (b) a separate axum REST/JSON layer next to the gRPC services (tonic's router is axum-based since 0.12/0.13).

## Recommendations

1. Upgrade to tonic 0.14.x / prost 0.14 now while the codebase is small. Switch `rustyrmm-proto` to `tonic-prost-build`, add `tonic-prost`.
2. Model Agent connectivity as one Agent-initiated bidi `Connect` RPC with a bounded mpsc per connected Agent, a registry in the Server, HTTP/2 keepalive on both sides plus an application heartbeat, and a reconnect loop with backoff in the Agent.
3. Decide separately whether the Operator UI uses tonic-web or REST; both are workable.

## Sources
- https://crates.io/api/v1/crates/{tonic,prost,tonic-build,tonic-prost,tonic-prost-build,tonic-web}
- https://github.com/hyperium/tonic/blob/master/CHANGELOG.md
- https://github.com/hyperium/tonic/releases/tag/v0.13.0 and v0.14.0
- https://github.com/hyperium/tonic/blob/master/examples/src/streaming/server.rs and client.rs
- https://docs.rs/tonic/latest/tonic/transport/server/struct.Server.html and .../channel/struct.Endpoint.html
- https://github.com/hyperium/tonic/blob/master/tonic-web/README.md
- https://github.com/grpc/grpc-web (README, Streaming Support)
- https://github.com/grpc/grpc/blob/master/doc/keepalive.md
