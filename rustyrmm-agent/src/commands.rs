//! Running Commands, per Grilling: Command execution limits.

use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use rustyrmm_proto::common::v1::{Command, CommandResult, Interpreter};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::Notify;

/// Output kept per stream; the rest is read and discarded so the script never blocks.
pub const OUTPUT_CAP: usize = 1 << 20;
/// How long finished results are kept for resending after a reconnect. Longer than the
/// Server's 5-minute "outcome unknown" grace, and a late result still completes those.
const RETAIN_RESULTS: Duration = Duration::from_secs(15 * 60);
/// After killing a timed-out tree, how long to wait for its output pipes to close.
const DRAIN_AFTER_KILL: Duration = Duration::from_secs(5);

/// Runs one Command and returns its result. Never fails: problems become the result.
pub async fn run(state_dir: &Path, cmd: Command) -> CommandResult {
    let started_at = now();
    let mut result = match execute(state_dir, &cmd).await {
        Ok(r) => r,
        Err(e) => CommandResult {
            stderr: format!("rustyrmm-agent could not run the Command: {e:#}").into_bytes(),
            ..Default::default()
        },
    };
    result.id = cmd.id;
    result.started_at = Some(started_at);
    result.finished_at = Some(now());
    result
}

async fn execute(state_dir: &Path, cmd: &Command) -> anyhow::Result<CommandResult> {
    let interpreter = Interpreter::try_from(cmd.interpreter).unwrap_or(Interpreter::Unspecified);
    let (program, extension, args) = invocation(interpreter)?;

    // The script goes to a file only the Agent's account can read (Windows' `cmd` limits a
    // command line to ~8K characters). The name is random: nothing from the Server is a path.
    let dir = state_dir.join("commands");
    create_private_dir(&dir)?;
    let script = dir.join(format!(
        "{}.{extension}",
        hex::encode(rand::random::<[u8; 8]>())
    ));
    write_private(&script, cmd.script.as_bytes())?;
    let _cleanup = RemoveOnDrop(script.clone());

    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0); // Its own group, so a timeout kills the whole tree.
    let mut child = command.spawn()?;
    let pid = child.id();
    let stdout = tokio::spawn(read_capped(child.stdout.take().expect("piped")));
    let stderr = tokio::spawn(read_capped(child.stderr.take().expect("piped")));

    let timeout = Duration::from_secs(cmd.timeout_seconds.max(1).into());
    let (exit_code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => (status?.code(), false),
        Err(_) => {
            if let Some(pid) = pid {
                kill_tree(pid).await;
            }
            let _ = child.wait().await;
            (None, true)
        }
    };
    // A finished script can leave children holding the pipes open; don't wait on them forever.
    let drain = if timed_out { DRAIN_AFTER_KILL } else { timeout };
    let (stdout, out_cut) = tokio::time::timeout(drain, stdout)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let (stderr, err_cut) = tokio::time::timeout(drain, stderr)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();

    Ok(CommandResult {
        exit_code,
        stdout,
        stderr,
        truncated: out_cut || err_cut,
        timed_out,
        ..Default::default()
    })
}

/// (program, script file extension, arguments before the script path).
fn invocation(
    interpreter: Interpreter,
) -> anyhow::Result<(&'static str, &'static str, &'static [&'static str])> {
    const PS_ARGS: &[&str] = &[
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ];
    Ok(match interpreter {
        Interpreter::Sh => ("sh", "sh", &[]),
        Interpreter::Bash => ("bash", "sh", &[]),
        Interpreter::Powershell if cfg!(windows) => ("powershell", "ps1", PS_ARGS),
        Interpreter::Powershell => ("pwsh", "ps1", PS_ARGS),
        Interpreter::Cmd if cfg!(windows) => ("cmd", "cmd", &["/D", "/C"]),
        Interpreter::Cmd => anyhow::bail!("cmd is only available on Windows"),
        Interpreter::Unspecified => anyhow::bail!("no interpreter given"),
    })
}

/// Reads a stream to the end, keeping at most OUTPUT_CAP bytes. Returns (kept, was cut).
async fn read_capped(mut reader: impl AsyncRead + Unpin) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut cut = false;
    let mut buf = [0u8; 8192];
    while let Ok(n @ 1..) = reader.read(&mut buf).await {
        let room = OUTPUT_CAP - kept.len();
        kept.extend_from_slice(&buf[..n.min(room)]);
        cut |= n > room;
    }
    (kept, cut)
}

#[cfg(unix)]
async fn kill_tree(pid: u32) {
    // The script leads its own process group; a negative pid signals the whole group.
    // SAFETY: kill() has no memory effects; pid comes from our own child.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(windows)]
async fn kill_tree(pid: u32) {
    // taskkill /T kills the process and every descendant.
    let _ = tokio::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output()
        .await;
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(contents)
}

struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn now() -> prost_types::Timestamp {
    prost_types::Timestamp::from(SystemTime::now())
}

/// Finished results, kept across Sessions so they can be resent after a reconnect
/// (the Server ignores duplicates by Command id).
#[derive(Default)]
pub struct Outbox {
    results: Mutex<Vec<(Instant, CommandResult)>>,
    pub added: Notify,
}

impl Outbox {
    pub fn push(&self, result: CommandResult) {
        self.results.lock().unwrap().push((Instant::now(), result));
        self.added.notify_one();
    }

    /// The results still retained, oldest first.
    pub fn retained(&self) -> Vec<CommandResult> {
        let mut results = self.results.lock().unwrap();
        results.retain(|(at, _)| at.elapsed() < RETAIN_RESULTS);
        results.iter().map(|(_, r)| r.clone()).collect()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn sh(script: &str, timeout_seconds: u32) -> CommandResult {
        let dir = tempfile::tempdir().unwrap();
        let r = run(
            dir.path(),
            Command {
                id: "c1".into(),
                interpreter: Interpreter::Sh as i32,
                script: script.into(),
                timeout_seconds,
            },
        )
        .await;
        let left = std::fs::read_dir(dir.path().join("commands"))
            .unwrap()
            .count();
        assert_eq!(left, 0, "script file must be deleted");
        r
    }

    #[tokio::test]
    async fn reports_output_and_exit_code() {
        let r = sh("echo out; echo err >&2; exit 3", 10).await;
        assert_eq!(r.id, "c1");
        assert_eq!(r.exit_code, Some(3));
        assert_eq!(r.stdout, b"out\n");
        assert_eq!(r.stderr, b"err\n");
        assert!(!r.timed_out && !r.truncated);
        assert!(r.started_at.is_some() && r.finished_at.is_some());
    }

    #[tokio::test]
    async fn caps_output_and_keeps_draining() {
        // Twice the cap: if reading stopped at the cap, the script would block and time out.
        let r = sh("head -c 2097152 /dev/zero", 30).await;
        assert_eq!(r.stdout.len(), OUTPUT_CAP);
        assert!(r.truncated);
        assert_eq!(r.exit_code, Some(0));
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_tree() {
        let marker = tempfile::tempdir().unwrap();
        let pid_file = marker.path().join("child.pid");
        let started = Instant::now();
        let r = sh(
            &format!(
                "sleep 60 & echo $! > {}; echo partial; wait",
                pid_file.display()
            ),
            1,
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(15));
        assert!(r.timed_out);
        assert_eq!(r.exit_code, None);
        assert_eq!(r.stdout, b"partial\n", "output before the timeout is kept");
        let child: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Killed children linger briefly as zombies until init reaps them, so poll.
        // SAFETY: signal 0 only checks whether the process exists.
        let alive = || unsafe { libc::kill(child, 0) } == 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(), "the background child must be killed too");
    }

    #[tokio::test]
    async fn unsupported_interpreter_becomes_an_error_result() {
        let dir = tempfile::tempdir().unwrap();
        let r = run(
            dir.path(),
            Command {
                id: "c2".into(),
                interpreter: Interpreter::Cmd as i32,
                script: "echo hi".into(),
                timeout_seconds: 5,
            },
        )
        .await;
        assert_eq!(r.id, "c2");
        assert_eq!(r.exit_code, None);
        assert!(String::from_utf8_lossy(&r.stderr).contains("only available on Windows"));
    }

    #[test]
    fn outbox_keeps_results_for_resending() {
        let outbox = Outbox::default();
        outbox.push(CommandResult {
            id: "a".into(),
            ..Default::default()
        });
        outbox.push(CommandResult {
            id: "b".into(),
            ..Default::default()
        });
        let ids: Vec<String> = outbox.retained().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(outbox.retained().len(), 2, "reading does not consume");
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[tokio::test]
    async fn runs_cmd_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let r = run(
            dir.path(),
            Command {
                id: "w1".into(),
                interpreter: Interpreter::Cmd as i32,
                script: "@echo off\r\necho hello\r\nexit /b 4\r\n".into(),
                timeout_seconds: 30,
            },
        )
        .await;
        assert_eq!(
            r.exit_code,
            Some(4),
            "stderr: {}",
            String::from_utf8_lossy(&r.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "hello");
    }
}
