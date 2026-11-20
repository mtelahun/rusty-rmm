mod output;
mod session;
mod tunnel;

use std::io::Read;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use rustyrmm_proto::common::v1::Interpreter;
use rustyrmm_proto::operator::v1::*;
use tonic::Status;

use output::{bytes, details, parse_duration, time, Table};
use session::{connect, server_url, Api, Saved};

#[derive(Parser)]
#[command(name = "rustyrmm", version, about = "RustyRMM Operator CLI")]
struct Cli {
    /// Print JSON instead of tables.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Log in to a Server's Operator port and save the session.
    Login {
        /// The Operator port, e.g. https://rmm.example.com:8443
        server_url: String,
        #[arg(long)]
        username: Option<String>,
        /// Read the password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
        /// Also trust this CA certificate (PEM), for Servers without a public certificate.
        #[arg(long)]
        ca_cert: Option<std::path::PathBuf>,
    },
    /// End the saved session.
    Logout,
    /// Environments.
    Env {
        #[command(subcommand)]
        command: EnvCmd,
    },
    /// Pairing Tokens for adding Endpoints to an Environment.
    PairingToken {
        #[command(subcommand)]
        command: TokenCmd,
    },
    /// Endpoints.
    Endpoint {
        #[command(subcommand)]
        command: EndpointCmd,
    },
    /// Recent Samples (Monitoring) of an Endpoint.
    Samples {
        /// Endpoint id or hostname.
        endpoint: String,
        #[arg(long, default_value = "1h")]
        since: String,
    },
    /// The live process list of an Endpoint.
    Ps {
        /// Endpoint id or hostname.
        endpoint: String,
    },
    /// Run a Command on an Endpoint and wait for its result. Exits with the remote exit code.
    Run {
        /// Endpoint id or hostname.
        endpoint: String,
        /// Defaults to powershell on Windows Endpoints, sh elsewhere.
        #[arg(long, value_enum)]
        interpreter: Option<Shell>,
        /// The script to run.
        #[arg(short = 'c', conflicts_with = "file", required_unless_present = "file")]
        script: Option<String>,
        /// Read the script from a file.
        #[arg(short = 'f')]
        file: Option<std::path::PathBuf>,
        /// Seconds before the Agent kills it (default 300, max 3600).
        #[arg(long, default_value_t = 0)]
        timeout: u32,
    },
    /// Commands that were run.
    Command {
        #[command(subcommand)]
        command: CommandCmd,
    },
    /// Tunnels from a local port to a port on an Endpoint's loopback.
    Tunnel {
        #[command(subcommand)]
        command: TunnelCmd,
    },
    /// Operators.
    Operator {
        #[command(subcommand)]
        command: OperatorCmd,
    },
}

#[derive(Subcommand)]
enum EnvCmd {
    List,
    Create { name: String },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a token and print the Agent install one-liners (the token is shown only once).
    Create {
        /// Environment name or id.
        environment: String,
        #[arg(long, default_value = "7d")]
        expires: String,
    },
    List {
        /// Environment name or id.
        environment: String,
    },
    Revoke {
        id: String,
    },
}

#[derive(Subcommand)]
enum EndpointCmd {
    List {
        /// Only this Environment (name or id).
        #[arg(long)]
        env: Option<String>,
    },
    Show {
        endpoint: String,
    },
    Revoke {
        endpoint: String,
    },
    Delete {
        endpoint: String,
    },
}

#[derive(Subcommand)]
enum CommandCmd {
    List {
        /// Only this Endpoint (id or hostname).
        #[arg(long)]
        ep: Option<String>,
    },
    Show {
        id: String,
    },
}

#[derive(Subcommand)]
enum TunnelCmd {
    /// Listen locally and relay each connection to the Endpoint's 127.0.0.1:<port>. Runs until Ctrl-C.
    Open {
        /// Endpoint id or hostname.
        endpoint: String,
        /// The port on the Endpoint.
        port: u32,
        /// Local port to listen on (0 picks a free one).
        #[arg(long, default_value_t = 0)]
        listen_port: u16,
        /// Local address to listen on.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    /// Your open Tunnels.
    List,
    Close {
        tunnel_id: String,
    },
}

#[derive(Subcommand)]
enum OperatorCmd {
    /// Allow or deny an Operator opening Tunnels.
    Tunnels {
        username: String,
        #[arg(long, required_unless_present = "deny")]
        allow: bool,
        #[arg(long, conflicts_with = "allow")]
        deny: bool,
    },
    /// Add an Operator; prompts for their password.
    Add {
        username: String,
        #[arg(long)]
        password_stdin: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Sh,
    Bash,
    Powershell,
    Cmd,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let json = cli.json;
    match cli.command {
        Cmd::Login {
            server_url: url,
            username,
            password_stdin,
            ca_cert,
        } => {
            let server = server_url(&url);
            let ca_cert_pem = ca_cert.map(std::fs::read_to_string).transpose()?;
            let username = match username {
                Some(u) => u,
                None => prompt("Username: ")?,
            };
            let password = password(password_stdin, "Password: ")?;
            let mut client = connect(&server, ca_cert_pem.as_deref()).await?;
            let resp = client
                .login(LoginRequest { username, password })
                .await
                .map_err(status)?
                .into_inner();
            Saved {
                server,
                token: resp.session_token,
                ca_cert_pem,
            }
            .save()?;
            println!("Logged in until {}", time(resp.expires_at.as_ref()));
        }
        Cmd::Logout => {
            if let Ok(mut api) = Api::from_saved().await {
                // The session ends even if the Server is unreachable: the token is deleted here.
                let _ = api.client.logout(api.req(LogoutRequest {})).await;
            }
            Saved::remove()?;
            println!("Logged out");
        }
        Cmd::Env { command } => {
            let mut api = Api::from_saved().await?;
            match command {
                EnvCmd::List => {
                    let mut t = Table::new(&["name", "id", "created"]);
                    for e in environments(&mut api).await? {
                        t.row(vec![e.name, e.id, time(e.created_at.as_ref())]);
                    }
                    println!("{}", t.render(json));
                }
                EnvCmd::Create { name } => {
                    let req = api.req(CreateEnvironmentRequest { name });
                    let e = api
                        .client
                        .create_environment(req)
                        .await
                        .map_err(status)?
                        .into_inner();
                    println!("{}", details(&[("name", e.name), ("id", e.id)], json));
                }
            }
        }
        Cmd::PairingToken { command } => {
            let mut api = Api::from_saved().await?;
            match command {
                TokenCmd::Create {
                    environment,
                    expires,
                } => {
                    let environment_id = environment_id(&mut api, &environment).await?;
                    let ttl = parse_duration(&expires)?;
                    let ttl = Some(prost_types::Duration {
                        seconds: ttl.num_seconds(),
                        nanos: 0,
                    });
                    let req = api.req(CreatePairingTokenRequest {
                        environment_id,
                        ttl,
                    });
                    let r = api
                        .client
                        .create_pairing_token(req)
                        .await
                        .map_err(status)?
                        .into_inner();
                    let token = r.pairing_token.unwrap_or_default();
                    let pair = format!(
                        "pair {} --server {} --ca-fingerprint {} --install-service",
                        r.secret, r.agent_address, r.ca_fingerprint_sha256
                    );
                    let releases =
                        concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/latest/download");
                    let unix = format!("curl --proto '=https' --tlsv1.2 -LsSf {releases}/rustyrmm-agent-installer.sh | sh && sudo ~/.cargo/bin/rustyrmm-agent {pair}");
                    let windows = format!("powershell -ExecutionPolicy Bypass -c \"[Net.ServicePointManager]::SecurityProtocol = 'Tls12'; irm {releases}/rustyrmm-agent-installer.ps1 | iex\"; & \"$env:USERPROFILE\\.cargo\\bin\\rustyrmm-agent.exe\" {pair}");
                    println!(
                        "{}",
                        details(
                            &[
                                ("id", token.id),
                                ("expires", time(token.expires_at.as_ref())),
                                ("secret", r.secret),
                                ("install (Linux/macOS)", unix),
                                ("install (Windows, admin PowerShell)", windows),
                            ],
                            json,
                        )
                    );
                    if !json {
                        eprintln!("The secret is shown only once.");
                    }
                }
                TokenCmd::List { environment } => {
                    let environment_id = environment_id(&mut api, &environment).await?;
                    let mut t = Table::new(&["id", "created", "expires", "revoked"]);
                    let mut page_token = String::new();
                    loop {
                        let req = api.req(ListPairingTokensRequest {
                            environment_id: environment_id.clone(),
                            page_size: 0,
                            page_token,
                        });
                        let r = api
                            .client
                            .list_pairing_tokens(req)
                            .await
                            .map_err(status)?
                            .into_inner();
                        for p in r.pairing_tokens {
                            t.row(vec![
                                p.id,
                                time(p.created_at.as_ref()),
                                time(p.expires_at.as_ref()),
                                yes_no(p.revoked),
                            ]);
                        }
                        if r.next_page_token.is_empty() {
                            break;
                        }
                        page_token = r.next_page_token;
                    }
                    println!("{}", t.render(json));
                }
                TokenCmd::Revoke { id } => {
                    api.client
                        .revoke_pairing_token(api.req(RevokePairingTokenRequest { id }))
                        .await
                        .map_err(status)?;
                    println!("Revoked");
                }
            }
        }
        Cmd::Endpoint { command } => {
            let mut api = Api::from_saved().await?;
            match command {
                EndpointCmd::List { env } => {
                    let environment_id = match env {
                        Some(e) => Some(environment_id(&mut api, &e).await?),
                        None => None,
                    };
                    let mut t =
                        Table::new(&["hostname", "id", "online", "os", "last seen", "revoked"]);
                    for e in endpoints(&mut api, environment_id).await? {
                        let os = e
                            .inventory
                            .as_ref()
                            .and_then(|i| i.os.as_ref())
                            .map(|o| o.name.clone())
                            .unwrap_or_default();
                        t.row(vec![
                            e.hostname,
                            e.id,
                            yes_no(e.online),
                            os,
                            time(e.last_seen_at.as_ref()),
                            yes_no(e.revoked),
                        ]);
                    }
                    println!("{}", t.render(json));
                }
                EndpointCmd::Show { endpoint } => {
                    let e = endpoint_by_ref(&mut api, &endpoint).await?;
                    println!("{}", details(&endpoint_fields(e), json));
                }
                EndpointCmd::Revoke { endpoint } => {
                    let id = endpoint_by_ref(&mut api, &endpoint).await?.id;
                    api.client
                        .revoke_endpoint(api.req(RevokeEndpointRequest { id }))
                        .await
                        .map_err(status)?;
                    println!("Revoked");
                }
                EndpointCmd::Delete { endpoint } => {
                    let id = endpoint_by_ref(&mut api, &endpoint).await?.id;
                    api.client
                        .delete_endpoint(api.req(DeleteEndpointRequest { id }))
                        .await
                        .map_err(status)?;
                    println!("Deleted");
                }
            }
        }
        Cmd::Samples { endpoint, since } => {
            let mut api = Api::from_saved().await?;
            let endpoint_id = endpoint_by_ref(&mut api, &endpoint).await?.id;
            let end = chrono::Utc::now();
            let start = end - parse_duration(&since)?;
            let ts = |t: chrono::DateTime<chrono::Utc>| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: 0,
            };
            let mut t = Table::new(&[
                "time",
                "cpu %",
                "memory",
                "disk read/s",
                "disk write/s",
                "net rx/s",
                "net tx/s",
            ]);
            let mut page_token = String::new();
            loop {
                let req = api.req(ListSamplesRequest {
                    endpoint_id: endpoint_id.clone(),
                    start: Some(ts(start)),
                    end: Some(ts(end)),
                    page_size: 0,
                    page_token,
                });
                let r = api
                    .client
                    .list_samples(req)
                    .await
                    .map_err(status)?
                    .into_inner();
                for s in r.samples {
                    let sum = |f: fn(&rustyrmm_proto::common::v1::DiskSample) -> u64| {
                        s.disks.iter().map(f).sum::<u64>()
                    };
                    let net = |f: fn(&rustyrmm_proto::common::v1::InterfaceSample) -> u64| {
                        s.interfaces.iter().map(f).sum::<u64>()
                    };
                    t.row(vec![
                        time(s.collected_at.as_ref()),
                        format!("{:.1}", s.cpu_usage_percent),
                        format!(
                            "{} / {}",
                            bytes(s.memory_used_bytes),
                            bytes(s.memory_total_bytes)
                        ),
                        bytes(sum(|d| d.read_bytes_per_sec)),
                        bytes(sum(|d| d.write_bytes_per_sec)),
                        bytes(net(|i| i.rx_bytes_per_sec)),
                        bytes(net(|i| i.tx_bytes_per_sec)),
                    ]);
                }
                if r.next_page_token.is_empty() {
                    break;
                }
                page_token = r.next_page_token;
            }
            println!("{}", t.render(json));
        }
        Cmd::Ps { endpoint } => {
            let mut api = Api::from_saved().await?;
            let endpoint_id = endpoint_by_ref(&mut api, &endpoint).await?.id;
            let r = api
                .client
                .get_process_list(api.req(GetProcessListRequest { endpoint_id }))
                .await
                .map_err(status)?
                .into_inner();
            let mut t = Table::new(&["pid", "ppid", "name", "cpu %", "memory", "command"]);
            for p in r.processes {
                let ppid = p.ppid.map(|p| p.to_string()).unwrap_or_default();
                t.row(vec![
                    p.pid.to_string(),
                    ppid,
                    p.name,
                    format!("{:.1}", p.cpu_usage_percent),
                    bytes(p.memory_bytes),
                    p.command_line.join(" "),
                ]);
            }
            println!("{}", t.render(json));
        }
        Cmd::Run {
            endpoint,
            interpreter,
            script,
            file,
            timeout,
        } => {
            let mut api = Api::from_saved().await?;
            let ep = endpoint_by_ref(&mut api, &endpoint).await?;
            let script = match (script, file) {
                (Some(s), _) => s,
                (None, Some(f)) => std::fs::read_to_string(f)?,
                (None, None) => unreachable!("clap requires -c or -f"),
            };
            let windows = ep
                .inventory
                .as_ref()
                .and_then(|i| i.os.as_ref())
                .is_some_and(|o| o.family == "windows");
            let interpreter = match interpreter {
                Some(Shell::Sh) => Interpreter::Sh,
                Some(Shell::Bash) => Interpreter::Bash,
                Some(Shell::Powershell) => Interpreter::Powershell,
                Some(Shell::Cmd) => Interpreter::Cmd,
                None if windows => Interpreter::Powershell,
                None => Interpreter::Sh,
            };
            let req = api.req(RunCommandRequest {
                endpoint_id: ep.id,
                interpreter: interpreter as i32,
                script,
                timeout_seconds: timeout,
            });
            let id = api
                .client
                .run_command(req)
                .await
                .map_err(status)?
                .into_inner()
                .command_id;
            return wait_for(&mut api, id, json).await;
        }
        Cmd::Command { command } => {
            let mut api = Api::from_saved().await?;
            match command {
                CommandCmd::List { ep } => {
                    let endpoint_id = match ep {
                        Some(e) => Some(endpoint_by_ref(&mut api, &e).await?.id),
                        None => None,
                    };
                    let req = api.req(ListCommandsRequest {
                        endpoint_id,
                        page_size: 50,
                        page_token: String::new(),
                    });
                    let r = api
                        .client
                        .list_commands(req)
                        .await
                        .map_err(status)?
                        .into_inner();
                    let mut t =
                        Table::new(&["id", "created", "endpoint", "operator", "status", "exit"]);
                    for c in r.commands {
                        let exit = c.result.as_ref().map(exit_text).unwrap_or_default();
                        t.row(vec![
                            c.id,
                            time(c.created_at.as_ref()),
                            c.endpoint_hostname,
                            c.operator_username,
                            status_text(c.status),
                            exit,
                        ]);
                    }
                    println!("{}", t.render(json));
                }
                CommandCmd::Show { id } => {
                    let c = api
                        .client
                        .get_command(api.req(GetCommandRequest { id }))
                        .await
                        .map_err(status)?
                        .into_inner();
                    let r = c.result.clone().unwrap_or_default();
                    println!(
                        "{}",
                        details(
                            &[
                                ("id", c.id),
                                ("endpoint", c.endpoint_hostname),
                                ("operator", c.operator_username),
                                ("status", status_text(c.status)),
                                ("exit", exit_text(&r)),
                                ("created", time(c.created_at.as_ref())),
                                ("script", c.script),
                                ("stdout", String::from_utf8_lossy(&r.stdout).into_owned()),
                                ("stderr", String::from_utf8_lossy(&r.stderr).into_owned()),
                            ],
                            json,
                        )
                    );
                }
            }
        }
        Cmd::Tunnel {
            command:
                TunnelCmd::Open {
                    endpoint,
                    port,
                    listen_port,
                    bind,
                },
        } => {
            let mut api = Api::from_saved().await?;
            let endpoint_id = endpoint_by_ref(&mut api, &endpoint).await?.id;
            tunnel::open(&mut api, endpoint_id, port, &bind, listen_port, json).await?;
        }
        Cmd::Tunnel {
            command: TunnelCmd::List,
        } => {
            let mut api = Api::from_saved().await?;
            let r = api
                .client
                .list_tunnels(api.req(ListTunnelsRequest {}))
                .await
                .map_err(status)?
                .into_inner();
            let mut t = Table::new(&["id", "endpoint", "port", "opened"]);
            for tunnel in r.tunnels {
                t.row(vec![
                    tunnel.id,
                    tunnel.endpoint_id,
                    tunnel.port.to_string(),
                    time(tunnel.opened_at.as_ref()),
                ]);
            }
            println!("{}", t.render(json));
        }
        Cmd::Tunnel {
            command: TunnelCmd::Close { tunnel_id },
        } => {
            let mut api = Api::from_saved().await?;
            let req = api.req(CloseTunnelRequest {
                tunnel_id: tunnel_id.clone(),
            });
            api.client.close_tunnel(req).await.map_err(status)?;
            println!("Closed Tunnel {tunnel_id}");
        }
        Cmd::Operator {
            command:
                OperatorCmd::Tunnels {
                    username,
                    allow,
                    deny: _,
                },
        } => {
            let mut api = Api::from_saved().await?;
            let req = api.req(SetTunnelPermissionRequest {
                username: username.clone(),
                allowed: allow,
            });
            api.client
                .set_tunnel_permission(req)
                .await
                .map_err(status)?;
            let verb = if allow { "may now" } else { "may no longer" };
            println!("{username} {verb} open Tunnels");
        }
        Cmd::Operator {
            command:
                OperatorCmd::Add {
                    username,
                    password_stdin,
                },
        } => {
            let mut api = Api::from_saved().await?;
            let password = password(password_stdin, &format!("Password for {username}: "))?;
            let req = api.req(AddOperatorRequest {
                username: username.clone(),
                password,
            });
            let r = api
                .client
                .add_operator(req)
                .await
                .map_err(status)?
                .into_inner();
            println!("Added Operator {username} ({})", r.operator_id);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Polls until the Command finishes, prints its output, and returns its exit code.
async fn wait_for(api: &mut Api, id: String, json: bool) -> anyhow::Result<ExitCode> {
    loop {
        let c = api
            .client
            .get_command(api.req(GetCommandRequest { id: id.clone() }))
            .await
            .map_err(status)?
            .into_inner();
        if c.status == CommandStatus::OutcomeUnknown as i32 {
            anyhow::bail!(
                "Command {id}: outcome unknown (no result arrived; the Agent may have restarted)"
            );
        }
        if let Some(r) = c
            .result
            .filter(|_| c.status == CommandStatus::Completed as i32)
        {
            if json {
                println!(
                    "{}",
                    details(
                        &[
                            ("id", id),
                            ("exit", exit_text(&r)),
                            ("stdout", String::from_utf8_lossy(&r.stdout).into_owned()),
                            ("stderr", String::from_utf8_lossy(&r.stderr).into_owned()),
                        ],
                        true,
                    )
                );
            } else {
                use std::io::Write;
                std::io::stdout().write_all(&r.stdout)?;
                std::io::stderr().write_all(&r.stderr)?;
                if r.truncated {
                    eprintln!("[rustyrmm: output was cut off at 1 MiB per stream]");
                }
                if r.timed_out {
                    eprintln!("[rustyrmm: timed out; the Command was killed]");
                }
            }
            // 124 for a timeout, like `timeout(1)`; 1 if killed without an exit code.
            let code = match (r.timed_out, r.exit_code) {
                (true, _) => 124,
                (false, Some(code)) => code.clamp(0, 255),
                (false, None) => 1,
            };
            return Ok(ExitCode::from(code as u8));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn environments(api: &mut Api) -> anyhow::Result<Vec<Environment>> {
    let mut all = Vec::new();
    let mut page_token = String::new();
    loop {
        let req = api.req(ListEnvironmentsRequest {
            page_size: 0,
            page_token,
        });
        let r = api
            .client
            .list_environments(req)
            .await
            .map_err(status)?
            .into_inner();
        all.extend(r.environments);
        if r.next_page_token.is_empty() {
            return Ok(all);
        }
        page_token = r.next_page_token;
    }
}

/// An Environment given by name or id.
async fn environment_id(api: &mut Api, name_or_id: &str) -> anyhow::Result<String> {
    environments(api)
        .await?
        .into_iter()
        .find(|e| e.id == name_or_id || e.name == name_or_id)
        .map(|e| e.id)
        .ok_or_else(|| anyhow::anyhow!("no Environment named {name_or_id:?}"))
}

async fn endpoints(api: &mut Api, environment_id: Option<String>) -> anyhow::Result<Vec<Endpoint>> {
    let mut all = Vec::new();
    let mut page_token = String::new();
    loop {
        let req = api.req(ListEndpointsRequest {
            environment_id: environment_id.clone(),
            page_size: 0,
            page_token,
        });
        let r = api
            .client
            .list_endpoints(req)
            .await
            .map_err(status)?
            .into_inner();
        all.extend(r.endpoints);
        if r.next_page_token.is_empty() {
            return Ok(all);
        }
        page_token = r.next_page_token;
    }
}

/// An Endpoint given by id, or by a hostname that only one Endpoint has.
async fn endpoint_by_ref(api: &mut Api, id_or_hostname: &str) -> anyhow::Result<Endpoint> {
    if uuid::Uuid::parse_str(id_or_hostname).is_ok() {
        let req = api.req(GetEndpointRequest {
            id: id_or_hostname.to_owned(),
        });
        return Ok(api
            .client
            .get_endpoint(req)
            .await
            .map_err(status)?
            .into_inner());
    }
    let mut matches: Vec<Endpoint> = endpoints(api, None)
        .await?
        .into_iter()
        .filter(|e| e.hostname == id_or_hostname)
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no Endpoint with hostname {id_or_hostname:?}"),
        1 => Ok(matches.remove(0)),
        _ => anyhow::bail!(
            "{} Endpoints have hostname {id_or_hostname:?}; use an id: {}",
            matches.len(),
            matches
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn endpoint_fields(e: Endpoint) -> Vec<(&'static str, String)> {
    let inv = e.inventory.unwrap_or_default();
    let os = inv.os.unwrap_or_default();
    let cpu = inv.cpu.unwrap_or_default();
    let updates = inv.updates.unwrap_or_default();
    let count = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into());
    let ips: Vec<String> = inv
        .interfaces
        .iter()
        .flat_map(|i| i.ipv4.iter().chain(&i.ipv6).cloned())
        .collect();
    vec![
        ("hostname", e.hostname),
        ("id", e.id),
        ("online", yes_no(e.online)),
        ("revoked", yes_no(e.revoked)),
        ("last seen", time(e.last_seen_at.as_ref())),
        ("paired", time(e.paired_at.as_ref())),
        ("os", format!("{} ({})", os.name, os.family)),
        (
            "virtualization",
            os.virt_system.unwrap_or_else(|| "unknown".into()),
        ),
        ("timezone", inv.timezone),
        (
            "cpu",
            format!(
                "{} ({} cores, {} threads)",
                cpu.brand, cpu.core_count, cpu.thread_count
            ),
        ),
        ("memory", bytes(inv.memory_total_bytes)),
        (
            "disks",
            inv.disks
                .iter()
                .map(|d| format!("{} {}", d.mount_point, bytes(d.total_bytes)))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        ("ip addresses", ips.join(", ")),
        (
            "updates",
            format!(
                "{} regular, {} security",
                count(updates.regular),
                count(updates.security)
            ),
        ),
        ("agent version", inv.agent_version),
    ]
}

fn exit_text(r: &rustyrmm_proto::common::v1::CommandResult) -> String {
    match (r.timed_out, r.exit_code) {
        (true, _) => "timed out".into(),
        (false, Some(c)) => c.to_string(),
        (false, None) => "killed".into(),
    }
}

fn status_text(status: i32) -> String {
    match CommandStatus::try_from(status) {
        Ok(CommandStatus::Dispatched) => "running",
        Ok(CommandStatus::Completed) => "completed",
        Ok(CommandStatus::OutcomeUnknown) => "outcome unknown",
        _ => "unknown",
    }
    .into()
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}

/// The Server's error message, without tonic's wrapping.
pub(crate) fn status(s: Status) -> anyhow::Error {
    anyhow::anyhow!("{} ({:?})", s.message(), s.code())
}

fn prompt(label: &str) -> anyhow::Result<String> {
    use std::io::Write;
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

fn password(from_stdin: bool, label: &str) -> anyhow::Result<String> {
    if from_stdin {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s.trim_end_matches(['\r', '\n']).to_owned())
    } else {
        Ok(rpassword::prompt_password(label)?)
    }
}
