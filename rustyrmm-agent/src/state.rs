//! The Agent's on-disk state after Pairing: Server details, key, certificate and the Server's CA.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default state directory for the OS.
pub fn default_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\RustyRMM")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/RustyRMM")
    } else {
        PathBuf::from("/var/lib/rustyrmm-agent")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The Agent port as `host:port`.
    pub server: String,
    pub endpoint_id: String,
}

/// Everything `run` needs, loaded from the state directory.
pub struct Paired {
    pub dir: PathBuf,
    pub config: Config,
    pub key_pem: String,
    pub cert_pem: String,
    pub ca_pem: String,
}

const CONFIG: &str = "agent.json";
const KEY: &str = "agent.key";
const CERT: &str = "agent.crt";
const CA: &str = "ca.crt";

impl Paired {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let read = |name: &str| {
            fs::read_to_string(dir.join(name)).map_err(|e| {
                anyhow::anyhow!(
                    "cannot read {} ({e}); has this Endpoint been paired?",
                    dir.join(name).display()
                )
            })
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            config: serde_json::from_str(&read(CONFIG)?)?,
            key_pem: read(KEY)?,
            cert_pem: read(CERT)?,
            ca_pem: read(CA)?,
        })
    }

    /// Writes all state. The key is written first and only readable by its owner.
    pub fn save(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.dir)?;
        write_private(&self.dir.join(KEY), self.key_pem.as_bytes())?;
        fs::write(self.dir.join(CA), &self.ca_pem)?;
        fs::write(
            self.dir.join(CONFIG),
            serde_json::to_string_pretty(&self.config)?,
        )?;
        self.save_cert(&self.cert_pem)
    }

    /// Replaces the certificate, e.g. after renewal. Atomic, so a crash never leaves it half-written.
    pub fn save_cert(&self, cert_pem: &str) -> anyhow::Result<()> {
        let tmp = self.dir.join(format!("{CERT}.tmp"));
        fs::write(&tmp, cert_pem)?;
        fs::rename(tmp, self.dir.join(CERT))?;
        Ok(())
    }
}

/// Writes a file readable only by its owner (root, as the Agent runs as a service).
// shortcut: on Windows this relies on ProgramData's default ACLs; tighten in Task: Agent service install.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(contents)
}
