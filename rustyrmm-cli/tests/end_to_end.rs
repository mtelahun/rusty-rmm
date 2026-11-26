//! The `rustyrmm` binary against the real Server (both ports) and a real paired Agent.
#![cfg(unix)]

use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Duration;

use rustyrmm_proto::agent::v1::agent_service_server::AgentServiceServer;
use rustyrmm_server::sessions::{Sessions, Settings};
use rustyrmm_server::{agent_service, ca::Ca, operator_service, operators};
use sqlx::PgPool;

const PASSWORD: &str = "correct horse battery";

struct Rig {
    operator_url: String,
    operator_ca: std::path::PathBuf,
    config: tempfile::TempDir,
    _state: tempfile::TempDir,
}

impl Rig {
    fn cli(&self, args: &[&str]) -> Output {
        self.cli_with_stdin(args, "")
    }

    fn cli_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustyrmm"))
            .args(args)
            .env("RUSTYRMM_CONFIG_DIR", self.config.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Runs the CLI with --json and parses stdout; panics with stderr on failure.
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.cli(&all);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

async fn start(pool: PgPool) -> Rig {
    let state = tempfile::tempdir().unwrap();
    let ca = Arc::new(Ca::load_or_create(state.path()).unwrap());
    let sessions = Arc::new(Sessions::default());
    let settings = Settings {
        sample_interval: Duration::from_millis(500),
        ..Settings::default()
    };

    let agent_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agent_addr = agent_listener.local_addr().unwrap();
    tokio::spawn(
        tonic::transport::Server::builder()
            .tls_config(agent_service::tls_config(&ca, vec!["127.0.0.1".into()]).unwrap())
            .unwrap()
            .add_service(AgentServiceServer::new(agent_service::AgentApi {
                pool: pool.clone(),
                ca: ca.clone(),
                sessions: sessions.clone(),
                settings,
            }))
            .serve_with_incoming(tonic::transport::server::TcpIncoming::from(agent_listener)),
    );

    // A self-signed certificate stands in for the Operator port's public-CA one.
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let operator_ca = state.path().join("operator-ca.pem");
    std::fs::write(&operator_ca, cert.pem()).unwrap();
    let operator_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = operator_listener.local_addr().unwrap().port();
    let api = operator_service::OperatorApi {
        pool: pool.clone(),
        sessions,
        agent_address: agent_addr.to_string(),
        ca_fingerprint: ca.fingerprint_sha256(),
        email_enabled: false,
    };
    tokio::spawn(operator_service::serve(
        api,
        operator_service::tls_config(&cert.pem(), &key.serialize_pem()),
        operator_listener,
    ));
    operators::add_operator(&pool, "admin", PASSWORD, "admin@example.com")
        .await
        .unwrap();
    Rig {
        operator_url: format!("https://localhost:{port}"),
        operator_ca,
        config: tempfile::tempdir().unwrap(),
        _state: state,
    }
}

/// Runs a blocking CLI call without stalling the in-process Server.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[sqlx::test(migrations = "../rustyrmm-server/migrations")]
async fn operator_pairs_an_agent_and_runs_a_command(pool: PgPool) {
    let rig = Arc::new(start(pool).await);

    let r = rig.clone();
    let login = blocking(move || {
        r.cli_with_stdin(
            &[
                "login",
                &r.operator_url,
                "--username",
                "admin",
                "--password-stdin",
                "--ca-cert",
                r.operator_ca.to_str().unwrap(),
            ],
            PASSWORD,
        )
    })
    .await;
    assert!(
        login.status.success(),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(rig.config.path().join("session.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the session token file is owner-only");
    }

    let r = rig.clone();
    let token = blocking(move || {
        r.json(&["env", "create", "lab", "--owner", "Acme"]);
        let set = r.json(&["env", "set-owner", "lab", "Beta"]);
        assert_eq!(set["owner"], "Beta");
        r.json(&["pairing-token", "create", "lab", "--expires", "1h"])
    })
    .await;
    let one_liner = token["install_(Linux/macOS)"].as_str().unwrap();
    assert!(
        one_liner.contains("rustyrmm-agent-installer.sh")
            && one_liner.contains("--install-service")
    );

    // Pair a real Agent with exactly what the one-liner carries.
    let words: Vec<&str> = one_liner.split_whitespace().collect();
    let arg = |flag: &str| words[words.iter().position(|w| *w == flag).unwrap() + 1].to_owned();
    let agent_dir = tempfile::tempdir().unwrap();
    rustyrmm_agent::pairing::pair(
        agent_dir.path(),
        &arg("--server"),
        token["secret"].as_str().unwrap(),
        &arg("--ca-fingerprint"),
    )
    .await
    .unwrap();
    let path = agent_dir.path().to_path_buf();
    let agent = tokio::spawn(async move { rustyrmm_agent::session::run(&path).await });

    // Wait until the Endpoint is Online and has reported its hostname.
    let mut hostname = String::new();
    for _ in 0..600 {
        let r = rig.clone();
        let list = blocking(move || r.json(&["endpoint", "list"])).await;
        if let Some(e) = list.as_array().unwrap().first() {
            if e["online"] == "yes" && !e["hostname"].as_str().unwrap().is_empty() {
                hostname = e["hostname"].as_str().unwrap().to_owned();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !hostname.is_empty(),
        "Endpoint never came Online with a hostname"
    );

    // The Agent samples every 500ms, so at least one Sample arrives shortly.
    let mut samples = 0;
    for _ in 0..100 {
        let (r, h) = (rig.clone(), hostname.clone());
        let list = blocking(move || r.json(&["samples", &h])).await;
        samples = list.as_array().unwrap().len();
        if samples > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(samples > 0, "Endpoint never reported a Sample");

    // `run` by hostname prints the output and exits with the remote exit code.
    let (r, h) = (rig.clone(), hostname.clone());
    let run =
        blocking(move || r.cli(&["run", &h, "-c", "echo hello from the Endpoint; exit 7"])).await;
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "hello from the Endpoint\n"
    );
    assert_eq!(run.status.code(), Some(7));

    let (r, h) = (rig.clone(), hostname.clone());
    let (ps, commands, show) = blocking(move || {
        (
            r.json(&["ps", &h]),
            r.json(&["command", "list", "--ep", &h]),
            r.json(&["endpoint", "show", &h]),
        )
    })
    .await;
    assert!(!ps.as_array().unwrap().is_empty());
    assert_eq!(commands[0]["exit"], "7");
    assert_eq!(commands[0]["operator"], "admin");
    assert_eq!(show["hostname"], hostname.as_str());

    let (r, h) = (rig.clone(), hostname.clone());
    let after = blocking(move || {
        assert!(r.cli(&["endpoint", "revoke", &h]).status.success());
        r.json(&["endpoint", "show", &h])
    })
    .await;
    assert_eq!(after["revoked"], "yes");
    assert_eq!(after["online"], "no");

    let r = rig.clone();
    let (logout, after_logout) =
        blocking(move || (r.cli(&["logout"]), r.cli(&["env", "list"]))).await;
    assert!(logout.status.success());
    assert!(
        !after_logout.status.success(),
        "commands need a session after logout"
    );
    agent.abort();
}
