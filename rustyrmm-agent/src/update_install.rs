//! Update Installs (Linux: apt, dnf). The package manager runs in its own transient systemd
//! unit (`systemd-run --unit=… --collect`), apart from the Agent, so an Agent restart never
//! stops it, and the Agent never kills it. Its output goes to a file in the state directory,
//! never back through systemd-run. The id and unit name are kept in the state directory: a
//! restarted Agent reattaches, and the outcome is resent until the Server acknowledges it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use prost::Message;
use rustyrmm_proto::agent::v1::{agent_message, AgentMessage, UpdateInstallStarted};
use rustyrmm_proto::common::v1::{
    UpdateInstallOutcome, UpdateInstallOutcomeKind, UpdateInstallRequest, UpdateResult,
    UpdateResultStatus, UpdateSelection, UpdateStatus,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify};

use crate::updates;

const STATE: &str = "update-install.json";
/// How often a running install unit is checked.
const POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Manager {
    Apt,
    Dnf,
}

/// The on-disk record of the current Update Install, written before the unit starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub unit: String,
    pub security: bool,
    pub manager: Manager,
    /// The selected packages pending at request time.
    pub pending: Vec<String>,
    /// Unix seconds.
    pub started_at: i64,
    /// The package manager's exit code, if it was seen before the unit was collected.
    pub exit_code: Option<i32>,
    /// The protobuf-encoded outcome (hex) once known; resent until acknowledged.
    pub outcome: Option<String>,
}

/// The state of an install unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Active,
    /// Ended but not yet collected, with the main process's exit code.
    Exited(Option<i32>),
    /// Collected (or never existed): the exit code is lost.
    Gone,
}

/// What an Update Install needs from the machine; tests use a fake.
pub trait Host: Send + Sync + 'static {
    /// The package manager and the names of the pending packages for the selection, or None if
    /// neither apt nor dnf can be checked.
    fn pending(
        &self,
        security: bool,
    ) -> impl Future<Output = Option<(Manager, Vec<String>)>> + Send;
    /// Whether the package manager's lock is held.
    fn locked(&self, manager: Manager) -> bool;
    /// Runs `systemd-run` with these arguments; returns once the unit is started.
    fn systemd_run(&self, args: &[String]) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn unit(&self, unit: &str) -> impl Future<Output = Unit> + Send;
    /// The read-only Update check (counts).
    fn check(&self) -> impl Future<Output = UpdateStatus> + Send;
}

/// The Agent's Update Install, shared across Sessions.
pub struct Installer<H> {
    dir: PathBuf,
    host: H,
    /// Signalled when an outcome is recorded and should be sent.
    pub ready: Notify,
}

impl<H: Host> Installer<H> {
    pub fn new(dir: &Path, host: H) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_path_buf(),
            host,
            ready: Notify::new(),
        })
    }

    /// Handles a request from the Server. "Started" and a busy refusal go to `tx` (the current
    /// Session); every other outcome is recorded and announced through `ready`.
    pub async fn request(
        self: Arc<Self>,
        req: UpdateInstallRequest,
        tx: mpsc::Sender<AgentMessage>,
    ) {
        let id = req.id.clone();
        match self.begin(req).await {
            Begin::Busy(outcome) => {
                let _ = tx
                    .send(message(agent_message::Message::UpdateInstallOutcome(
                        outcome,
                    )))
                    .await;
            }
            Begin::Recorded => self.ready.notify_one(),
            Begin::Started => {
                let started = UpdateInstallStarted { id };
                let _ = tx
                    .send(message(agent_message::Message::UpdateInstallStarted(
                        started,
                    )))
                    .await;
                drop(tx); // Don't hold the Session open while the install runs.
                self.watch(true).await;
            }
        }
    }

    /// After an Agent restart: waits for a still-running unit, or settles one that ended while
    /// the Agent was down. A recorded outcome is left for the Sessions to resend.
    pub async fn reattach(self: Arc<Self>) {
        self.watch(false).await;
    }

    /// The recorded outcome not yet acknowledged by the Server.
    pub fn unsent(&self) -> Option<UpdateInstallOutcome> {
        let hex = load(&self.dir)?.outcome?;
        UpdateInstallOutcome::decode(hex::decode(hex).ok()?.as_slice()).ok()
    }

    /// The Server stored the outcome: forget the Update Install.
    pub fn acknowledge(&self, id: &str) {
        if load(&self.dir).is_some_and(|r| r.id == id && r.outcome.is_some()) {
            let _ = std::fs::remove_file(self.dir.join(STATE));
            let _ = std::fs::remove_file(log_path(&self.dir, id));
        }
    }

    async fn begin(&self, req: UpdateInstallRequest) -> Begin {
        let now = SystemTime::now();
        if load(&self.dir).is_some() {
            return Begin::Busy(outcome(
                &req.id,
                UpdateInstallOutcomeKind::RejectedBusy,
                now,
            ));
        }
        let security = req.selection == UpdateSelection::Security as i32;
        let mut record = Record {
            unit: format!("rustyrmm-update-{}", req.id),
            id: req.id,
            security,
            manager: Manager::Apt,
            pending: Vec::new(),
            started_at: unix(now),
            exit_code: None,
            outcome: None,
        };
        // The id becomes part of a unit and file name: only a Server-made UUID is accepted.
        let valid_id =
            record.id.len() == 36 && record.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        let pending = if valid_id {
            self.host.pending(security).await
        } else {
            None
        };
        let Some((manager, pending)) = pending else {
            return self.record(
                record,
                UpdateInstallOutcomeKind::FailedToStart,
                Vec::new(),
                None,
            );
        };
        (record.manager, record.pending) = (manager, pending);
        if self.host.locked(manager) {
            return self.record(
                record,
                UpdateInstallOutcomeKind::RejectedBusy,
                Vec::new(),
                None,
            );
        }
        if record.pending.is_empty() {
            // Nothing to do; never run the package manager without packages (dnf would
            // upgrade everything).
            let check = self.host.check().await;
            return self.record(
                record,
                UpdateInstallOutcomeKind::Finished,
                Vec::new(),
                Some(check),
            );
        }

        if let Err(e) = save(&self.dir, &record) {
            eprintln!("Update Install {}: cannot write its state: {e}", record.id);
            return self.record(
                record,
                UpdateInstallOutcomeKind::FailedToStart,
                Vec::new(),
                None,
            );
        }
        let args = systemd_run_args(
            &record.unit,
            manager,
            &record.pending,
            &log_path(&self.dir, &record.id),
        );
        if let Err(e) = self.host.systemd_run(&args).await {
            eprintln!("Update Install {}: systemd-run failed: {e:#}", record.id);
            return self.record(
                record,
                UpdateInstallOutcomeKind::FailedToStart,
                Vec::new(),
                None,
            );
        }
        Begin::Started
    }

    /// Records an outcome; resent until acknowledged.
    fn record(
        &self,
        mut record: Record,
        kind: UpdateInstallOutcomeKind,
        results: Vec<UpdateResult>,
        updates: Option<UpdateStatus>,
    ) -> Begin {
        let mut o = outcome(&record.id, kind, SystemTime::now());
        o.started_at = Some(prost_types::Timestamp {
            seconds: record.started_at,
            nanos: 0,
        });
        o.results = results;
        o.updates = updates;
        record.outcome = Some(hex::encode(o.encode_to_vec()));
        if let Err(e) = save(&self.dir, &record) {
            eprintln!(
                "Update Install {}: cannot record its outcome: {e}",
                record.id
            );
        }
        Begin::Recorded
    }

    /// Waits for the unit to end, then records the outcome. `seen_active`: this Agent saw the
    /// unit running, so a vanished unit ended normally; otherwise its end was missed and,
    /// without an exit code, it is Interrupted.
    async fn watch(&self, mut seen_active: bool) {
        loop {
            let Some(mut record) = load(&self.dir).filter(|r| r.outcome.is_none()) else {
                return;
            };
            match self.host.unit(&record.unit).await {
                Unit::Active => {
                    seen_active = true;
                    tokio::time::sleep(POLL).await;
                    continue;
                }
                Unit::Exited(code) => {
                    record.exit_code = code;
                    seen_active = true;
                }
                Unit::Gone => {}
            }
            let after = if seen_active || record.exit_code.is_some() {
                self.host.pending(record.security).await
            } else {
                None
            };
            match after {
                Some((_, after)) => {
                    let results =
                        results(&record.pending, &record.pending, &after, record.exit_code);
                    let check = self.host.check().await;
                    self.record(
                        record,
                        UpdateInstallOutcomeKind::Finished,
                        results,
                        Some(check),
                    );
                }
                None => {
                    self.record(
                        record,
                        UpdateInstallOutcomeKind::Interrupted,
                        Vec::new(),
                        None,
                    );
                }
            }
            self.ready.notify_one();
            return;
        }
    }
}

enum Begin {
    /// Another Update Install is unfinished: refused without touching its state.
    Busy(UpdateInstallOutcome),
    Recorded,
    Started,
}

/// Each selected package's result: installed if pending before and not after, failed (with the
/// exit code, when known) if still pending, not pending if it wasn't pending at request time.
pub fn results(
    selection: &[String],
    before: &[String],
    after: &[String],
    exit_code: Option<i32>,
) -> Vec<UpdateResult> {
    selection
        .iter()
        .map(|package| {
            let (status, exit_code) = if !before.contains(package) {
                (UpdateResultStatus::NotPending, None)
            } else if after.contains(package) {
                (UpdateResultStatus::Failed, exit_code)
            } else {
                (UpdateResultStatus::Installed, None)
            };
            UpdateResult {
                package: package.clone(),
                status: status as i32,
                exit_code,
            }
        })
        .collect()
}

/// The `systemd-run` arguments: a transient unit that execs the package manager directly (no
/// shell or script), collected when it ends, with output to `log`. Never `--pipe` or `--wait`:
/// the Agent must not be tied to the install.
pub fn systemd_run_args(
    unit: &str,
    manager: Manager,
    packages: &[String],
    log: &Path,
) -> Vec<String> {
    let mut args = vec![
        format!("--unit={unit}"),
        "--collect".into(),
        format!("--property=StandardOutput=file:{}", log.display()),
        "--setenv=DEBIAN_FRONTEND=noninteractive".into(),
    ];
    let command: &[&str] = match manager {
        Manager::Apt => &[
            "apt-get",
            "-y",
            "-o",
            "Dpkg::Options::=--force-confold",
            "install",
        ],
        Manager::Dnf => &["dnf", "-y", "upgrade"],
    };
    args.extend(command.iter().map(|s| s.to_string()));
    args.extend(packages.iter().cloned());
    args
}

/// `systemctl show -p LoadState -p ActiveState -p ExecMainStatus` output. A collected unit
/// shows `LoadState=not-found` (with a meaningless `ExecMainStatus=0`).
pub fn parse_unit(out: &str) -> Unit {
    let field = |name: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_default()
    };
    if field("LoadState") != "loaded" {
        return Unit::Gone;
    }
    match field("ActiveState") {
        "active" | "activating" | "deactivating" | "reloading" => Unit::Active,
        _ => Unit::Exited(field("ExecMainStatus").parse().ok()),
    }
}

fn outcome(id: &str, kind: UpdateInstallOutcomeKind, finished: SystemTime) -> UpdateInstallOutcome {
    UpdateInstallOutcome {
        id: id.into(),
        kind: kind as i32,
        finished_at: Some(prost_types::Timestamp::from(finished)),
        ..Default::default()
    }
}

fn message(m: agent_message::Message) -> AgentMessage {
    AgentMessage { message: Some(m) }
}

fn unix(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn log_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("update-{id}.log"))
}

pub fn load(dir: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(dir.join(STATE)).ok()?).ok()
}

/// Atomic, so a crash never leaves the record half-written.
fn save(dir: &Path, record: &Record) -> std::io::Result<()> {
    let tmp = dir.join(format!("{STATE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
    std::fs::rename(tmp, dir.join(STATE))
}

/// The real machine.
pub struct System;

impl Host for System {
    async fn pending(&self, security: bool) -> Option<(Manager, Vec<String>)> {
        if !cfg!(target_os = "linux") {
            return None; // Linux only: the install runs in a systemd unit.
        }
        if let Some((0, out)) = updates::output("apt-get", &["-s", "-q", "dist-upgrade"]).await {
            return Some((Manager::Apt, updates::apt_packages(&out, security)));
        }
        // check-update exits 100 when updates exist.
        let (code @ (0 | 100), out) = updates::output("dnf", &["-q", "check-update"]).await? else {
            return None;
        };
        let all = if code == 0 {
            Vec::new()
        } else {
            updates::dnf_packages(&out)
        };
        if !security {
            return Some((Manager::Dnf, all));
        }
        let (0, sec) = updates::output("dnf", &["-q", "updateinfo", "list", "--security"]).await?
        else {
            return None;
        };
        let sec = updates::dnf_advisory_packages(&sec);
        Some((
            Manager::Dnf,
            all.into_iter().filter(|p| sec.contains(p)).collect(),
        ))
    }

    fn locked(&self, manager: Manager) -> bool {
        let paths: &[&str] = match manager {
            Manager::Apt => &["/var/lib/dpkg/lock-frontend", "/var/lib/dpkg/lock"],
            Manager::Dnf => &["/var/lib/rpm/.rpm.lock"],
        };
        paths.iter().any(|p| lock_held(Path::new(p)))
    }

    async fn systemd_run(&self, args: &[String]) -> anyhow::Result<()> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match updates::output("systemd-run", &args).await {
            Some((0, _)) => Ok(()),
            Some((code, _)) => anyhow::bail!("exit code {code}"),
            None => anyhow::bail!("could not run systemd-run"),
        }
    }

    async fn unit(&self, unit: &str) -> Unit {
        let unit = format!("{unit}.service");
        let args = [
            "show",
            &unit,
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            "-p",
            "ExecMainStatus",
        ];
        match updates::output("systemctl", &args).await {
            Some((0, out)) => parse_unit(&out),
            _ => Unit::Gone,
        }
    }

    async fn check(&self) -> UpdateStatus {
        updates::check().await
    }
}

/// Whether another process holds a write lock on the file. dpkg, apt and rpm use fcntl record
/// locks, which `flock` does not see, so this asks with `F_GETLK` without taking the lock.
#[cfg(unix)]
fn lock_held(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = std::fs::File::open(path) else {
        return false; // No lock file: nothing holds it.
    };
    // SAFETY: a zeroed flock is a valid value, and F_GETLK only writes into it.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    let r = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) };
    r == 0 && i32::from(lock.l_type) != libc::F_UNLCK
}

#[cfg(not(unix))]
fn lock_held(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const ID: &str = "0193a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    /// Pending lists are returned in order (before, after, ...); units likewise.
    #[derive(Default)]
    struct Fake {
        pending: Mutex<VecDeque<Vec<String>>>,
        units: Mutex<VecDeque<Unit>>,
        locked: bool,
        runs: Mutex<Vec<Vec<String>>>,
    }

    impl Fake {
        fn new(pending: &[&[&str]], units: &[Unit]) -> Self {
            Self {
                pending: Mutex::new(pending.iter().map(|p| names(p)).collect()),
                units: Mutex::new(units.iter().copied().collect()),
                ..Default::default()
            }
        }
    }

    impl Host for Fake {
        async fn pending(&self, _security: bool) -> Option<(Manager, Vec<String>)> {
            let next = self.pending.lock().unwrap().pop_front()?;
            Some((Manager::Apt, next))
        }
        fn locked(&self, _: Manager) -> bool {
            self.locked
        }
        async fn systemd_run(&self, args: &[String]) -> anyhow::Result<()> {
            self.runs.lock().unwrap().push(args.to_vec());
            Ok(())
        }
        async fn unit(&self, _: &str) -> Unit {
            self.units.lock().unwrap().pop_front().unwrap_or(Unit::Gone)
        }
        async fn check(&self) -> UpdateStatus {
            UpdateStatus {
                regular: Some(0),
                security: Some(0),
            }
        }
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn request() -> UpdateInstallRequest {
        UpdateInstallRequest {
            id: ID.into(),
            selection: UpdateSelection::All as i32,
            ..Default::default()
        }
    }

    fn kind(o: &UpdateInstallOutcome) -> UpdateInstallOutcomeKind {
        UpdateInstallOutcomeKind::try_from(o.kind).unwrap()
    }

    #[test]
    fn results_are_installed_failed_or_not_pending() {
        let r = results(
            &names(&["a", "b", "c"]),
            &names(&["a", "b"]),
            &names(&["b"]),
            Some(100),
        );
        let got: Vec<(&str, i32, Option<i32>)> = r
            .iter()
            .map(|r| (r.package.as_str(), r.status, r.exit_code))
            .collect();
        assert_eq!(
            got,
            [
                ("a", UpdateResultStatus::Installed as i32, None),
                ("b", UpdateResultStatus::Failed as i32, Some(100)),
                ("c", UpdateResultStatus::NotPending as i32, None),
            ]
        );
        let unknown = results(&names(&["b"]), &names(&["b"]), &names(&["b"]), None);
        assert_eq!(
            unknown[0].exit_code, None,
            "an unobserved exit code stays absent"
        );
    }

    #[test]
    fn systemd_run_execs_the_package_manager_in_a_collected_unit() {
        for (manager, program) in [(Manager::Apt, "apt-get"), (Manager::Dnf, "dnf")] {
            let args = systemd_run_args(
                "rustyrmm-update-x",
                manager,
                &names(&["openssl"]),
                Path::new("/var/lib/rustyrmm-agent/update-x.log"),
            );
            assert!(args.contains(&"--unit=rustyrmm-update-x".to_string()));
            assert!(args.contains(&"--collect".to_string()));
            let first_command = args.iter().find(|a| !a.starts_with("--")).unwrap();
            assert_eq!(first_command, program);
            assert_eq!(args.last().unwrap(), "openssl");
            for banned in ["--pipe", "--wait", "-P"] {
                assert!(!args.iter().any(|a| a == banned), "{banned} in {args:?}");
            }
            assert!(
                !args.iter().any(|a| a.ends_with("sh") || a.ends_with(".sh")),
                "no shell or script: {args:?}"
            );
        }
    }

    #[test]
    fn unit_state_comes_from_systemctl_show() {
        let gone = "LoadState=not-found\nActiveState=inactive\nExecMainStatus=0\n";
        assert_eq!(parse_unit(gone), Unit::Gone);
        let running = "LoadState=loaded\nActiveState=active\nExecMainStatus=0\n";
        assert_eq!(parse_unit(running), Unit::Active);
        let failed = "LoadState=loaded\nActiveState=failed\nExecMainStatus=100\n";
        assert_eq!(parse_unit(failed), Unit::Exited(Some(100)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sees_a_package_manager_lock() {
        use std::os::fd::AsRawFd;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock-frontend");
        assert!(!lock_held(&path), "no lock file");
        let file = std::fs::File::create(&path).unwrap();
        assert!(!lock_held(&path));
        // An open-file-description lock conflicts with record locks even in this process.
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_WRLCK as _;
        lock.l_whence = libc::SEEK_SET as _;
        assert_eq!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_OFD_SETLK, &mut lock) },
            0
        );
        assert!(lock_held(&path));
    }

    #[tokio::test]
    async fn install_runs_and_its_outcome_is_resent_until_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let host = Fake::new(
            &[&["openssl", "curl"], &["curl"]],
            &[Unit::Exited(Some(100))],
        );
        let installer = Installer::new(dir.path(), host);
        let (tx, mut rx) = mpsc::channel(4);
        installer.clone().request(request(), tx).await;
        let started = rx.recv().await.unwrap();
        assert!(matches!(
            started.message,
            Some(agent_message::Message::UpdateInstallStarted(ref s)) if s.id == ID
        ));
        assert_eq!(installer.host.runs.lock().unwrap().len(), 1);

        let outcome = installer.unsent().unwrap();
        assert_eq!(kind(&outcome), UpdateInstallOutcomeKind::Finished);
        let got: Vec<(&str, i32, Option<i32>)> = outcome
            .results
            .iter()
            .map(|r| (r.package.as_str(), r.status, r.exit_code))
            .collect();
        assert_eq!(
            got,
            [
                ("openssl", UpdateResultStatus::Installed as i32, None),
                ("curl", UpdateResultStatus::Failed as i32, Some(100)),
            ]
        );
        assert!(outcome.updates.is_some());

        // Every Session pass resends it until the Server's acknowledgement for this id.
        assert_eq!(installer.unsent(), Some(outcome.clone()));
        installer.acknowledge("another-id");
        assert_eq!(installer.unsent(), Some(outcome));
        installer.acknowledge(ID);
        assert_eq!(installer.unsent(), None);
        assert!(!dir.path().join(STATE).exists());
    }

    #[tokio::test]
    async fn a_second_request_or_a_held_lock_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let host = Fake::new(&[&["openssl"]], &[Unit::Active]);
        let installer = Installer::new(dir.path(), host);
        installer.begin(request()).await;
        let before = load(dir.path()).unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let mut second = request();
        second.id = "0193a1b2-c3d4-7e5f-8a9b-000000000002".into();
        installer.clone().request(second, tx).await;
        let reply = rx.recv().await.unwrap();
        let Some(agent_message::Message::UpdateInstallOutcome(o)) = reply.message else {
            panic!("expected an outcome, got {reply:?}");
        };
        assert_eq!(kind(&o), UpdateInstallOutcomeKind::RejectedBusy);
        assert_eq!(
            load(dir.path()).unwrap(),
            before,
            "the running one is untouched"
        );

        let dir = tempfile::tempdir().unwrap();
        let host = Fake {
            locked: true,
            ..Fake::new(&[&["openssl"]], &[])
        };
        let installer = Installer::new(dir.path(), host);
        installer.begin(request()).await;
        assert_eq!(
            kind(&installer.unsent().unwrap()),
            UpdateInstallOutcomeKind::RejectedBusy
        );
        assert!(installer.host.runs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn nothing_pending_finishes_without_running_anything() {
        let dir = tempfile::tempdir().unwrap();
        let installer = Installer::new(dir.path(), Fake::new(&[&[]], &[]));
        installer.begin(request()).await;
        let outcome = installer.unsent().unwrap();
        assert_eq!(kind(&outcome), UpdateInstallOutcomeKind::Finished);
        assert!(outcome.results.is_empty());
        assert!(installer.host.runs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reattach_after_a_restart() {
        let record = Record {
            id: ID.into(),
            unit: format!("rustyrmm-update-{ID}"),
            security: false,
            manager: Manager::Apt,
            pending: names(&["openssl"]),
            started_at: 0,
            exit_code: None,
            outcome: None,
        };

        // The unit ended while the Agent was down and nothing was recorded: Interrupted.
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &record).unwrap();
        let installer = Installer::new(dir.path(), Fake::new(&[&[]], &[Unit::Gone]));
        installer.clone().reattach().await;
        assert_eq!(
            kind(&installer.unsent().unwrap()),
            UpdateInstallOutcomeKind::Interrupted
        );

        // Still running: wait for it, then the fresh check decides.
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &record).unwrap();
        let installer = Installer::new(dir.path(), Fake::new(&[&[]], &[Unit::Exited(Some(0))]));
        installer.clone().reattach().await;
        let outcome = installer.unsent().unwrap();
        assert_eq!(kind(&outcome), UpdateInstallOutcomeKind::Finished);
        assert_eq!(
            outcome.results[0].status,
            UpdateResultStatus::Installed as i32
        );

        // An outcome already recorded is left alone, to be resent.
        installer.clone().reattach().await;
        assert_eq!(installer.unsent(), Some(outcome));
    }
}
