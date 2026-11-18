//! The saved login (Server, session token) and authenticated calls to the Operator port.

use std::path::PathBuf;

use rustyrmm_proto::operator::v1::operator_service_client::OperatorServiceClient;
use serde::{Deserialize, Serialize};
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::Request;

#[derive(Serialize, Deserialize)]
pub struct Saved {
    /// `https://host:port` of the Operator port.
    pub server: String,
    pub token: String,
    /// A private CA to trust in addition to the system roots (from `login --ca-cert`).
    pub ca_cert_pem: Option<String>,
}

/// `$RUSTYRMM_CONFIG_DIR/session.json`, else the OS config dir (e.g. `~/.config/rustyrmm`).
fn path() -> anyhow::Result<PathBuf> {
    let dir = match std::env::var_os("RUSTYRMM_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("no config directory on this system"))?
            .join("rustyrmm"),
    };
    Ok(dir.join("session.json"))
}

impl Saved {
    pub fn load() -> anyhow::Result<Self> {
        let path = path()?;
        let text = std::fs::read_to_string(&path)
            .map_err(|_| anyhow::anyhow!("not logged in; run `rustyrmm login <server-url>`"))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Saves the session readable only by the current user: it is a bearer credential.
    pub fn save(&self) -> anyhow::Result<()> {
        use std::io::Write;
        let path = path()?;
        std::fs::create_dir_all(path.parent().expect("has a parent"))?;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&path)?
            .write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn remove() -> anyhow::Result<()> {
        match std::fs::remove_file(path()?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// `host:port` or a full URL, as `https://…`.
pub fn server_url(server: &str) -> String {
    if server.contains("://") {
        server.to_owned()
    } else {
        format!("https://{server}")
    }
}

pub async fn connect(
    server: &str,
    ca_cert_pem: Option<&str>,
) -> anyhow::Result<OperatorServiceClient<Channel>> {
    let mut tls = ClientTlsConfig::new().with_native_roots();
    if let Some(pem) = ca_cert_pem {
        tls = tls.ca_certificate(Certificate::from_pem(pem));
    }
    let channel = Channel::from_shared(server.to_owned())?
        .tls_config(tls)?
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach {server}: {e}"))?;
    Ok(OperatorServiceClient::new(channel))
}

/// A logged-in client: every request carries the session token.
pub struct Api {
    pub client: OperatorServiceClient<Channel>,
    token: String,
}

impl Api {
    pub async fn from_saved() -> anyhow::Result<Self> {
        let saved = Saved::load()?;
        Ok(Self {
            client: connect(&saved.server, saved.ca_cert_pem.as_deref()).await?,
            token: saved.token,
        })
    }

    pub fn req<T>(&self, msg: T) -> Request<T> {
        let mut req = Request::new(msg);
        let value = format!("Bearer {}", self.token)
            .parse()
            .expect("hex token is valid metadata");
        req.metadata_mut().insert("authorization", value);
        req
    }
}
