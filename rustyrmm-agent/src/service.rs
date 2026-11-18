//! Installing the Agent as an OS service: systemd (Linux), launchd (macOS), SCM (Windows).
//! The binary is copied to a system location first, so the service never runs from a user's
//! home directory (where the dist installers put it).

use std::path::{Path, PathBuf};

use crate::state::Paired;

pub const SERVICE_NAME: &str = "rustyrmm-agent";
pub const LAUNCHD_LABEL: &str = "com.rustyrmm.agent";
pub const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/rustyrmm-agent.service";
pub const LAUNCHD_PLIST_PATH: &str = "/Library/LaunchDaemons/com.rustyrmm.agent.plist";

/// Where the service's copy of the binary lives.
pub fn installed_binary() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\Program Files\RustyRMM\rustyrmm-agent.exe")
    } else {
        PathBuf::from("/usr/local/bin/rustyrmm-agent")
    }
}

/// Installs and starts the service for a paired Agent. Needs root / Administrator.
pub fn install(state_dir: &Path) -> anyhow::Result<()> {
    Paired::load(state_dir)?; // Only a paired Agent can run as a service.
    let exe = installed_binary();
    copy_self_to(&exe)?;
    os::install(&exe, state_dir)
}

/// Stops and removes the service and the installed binary. Pairing state is kept, so
/// reinstalling resumes as the same Endpoint.
pub fn uninstall() -> anyhow::Result<()> {
    os::uninstall()?;
    match std::fs::remove_file(installed_binary()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn copy_self_to(dest: &Path) -> anyhow::Result<()> {
    let current = std::env::current_exe()?;
    if current.canonicalize().ok() == dest.canonicalize().ok() {
        return Ok(());
    }
    std::fs::create_dir_all(dest.parent().expect("absolute path"))?;
    // Copy then rename, so a running copy is replaced atomically on Unix.
    let tmp = dest.with_extension("new");
    std::fs::copy(&current, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// The systemd unit. The Agent runs as root: it manages the machine.
pub fn systemd_unit(exe: &Path, state_dir: &Path) -> String {
    format!(
        "[Unit]
Description=RustyRMM Agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=\"{}\" --state-dir \"{}\" run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
",
        exe.display(),
        state_dir.display()
    )
}

/// The launchd daemon plist (system-wide, runs at boot without a logged-in user).
pub fn launchd_plist(exe: &Path, state_dir: &Path) -> String {
    let arg = |s: &str| format!("        <string>{}</string>\n", xml_escape(s));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{}{}{}{}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        arg(&exe.display().to_string()),
        arg("--state-dir"),
        arg(&state_dir.display().to_string()),
        arg("run"),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Runs a system command, failing with its output if it exits non-zero.
fn run_cmd(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new(program).args(args).output()?;
    anyhow::ensure!(
        out.status.success(),
        "`{program} {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    use super::*;

    pub fn install(exe: &Path, state_dir: &Path) -> anyhow::Result<()> {
        std::fs::write(SYSTEMD_UNIT_PATH, systemd_unit(exe, state_dir))?;
        run_cmd("systemctl", &["daemon-reload"])?;
        run_cmd("systemctl", &["enable", "--now", SERVICE_NAME])
    }

    pub fn uninstall() -> anyhow::Result<()> {
        if Path::new(SYSTEMD_UNIT_PATH).exists() {
            run_cmd("systemctl", &["disable", "--now", SERVICE_NAME])?;
            std::fs::remove_file(SYSTEMD_UNIT_PATH)?;
            run_cmd("systemctl", &["daemon-reload"])?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::*;

    pub fn install(exe: &Path, state_dir: &Path) -> anyhow::Result<()> {
        std::fs::write(LAUNCHD_PLIST_PATH, launchd_plist(exe, state_dir))?;
        run_cmd("launchctl", &["bootstrap", "system", LAUNCHD_PLIST_PATH])
    }

    pub fn uninstall() -> anyhow::Result<()> {
        if Path::new(LAUNCHD_PLIST_PATH).exists() {
            run_cmd(
                "launchctl",
                &["bootout", &format!("system/{LAUNCHD_LABEL}")],
            )?;
            std::fs::remove_file(LAUNCHD_PLIST_PATH)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use os::run_as_service;

#[cfg(windows)]
mod os {
    use super::*;
    use std::ffi::OsString;
    use std::sync::OnceLock;
    use std::time::Duration;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    // Well-known SIDs (locale-independent): LocalSystem and Administrators.
    const SYSTEM_SID: &str = "*S-1-5-18";
    const ADMINS_SID: &str = "*S-1-5-32-544";

    pub fn install(exe: &Path, state_dir: &Path) -> anyhow::Result<()> {
        // The state directory holds the Agent key: SYSTEM and Administrators only.
        let dir = state_dir.display().to_string();
        let grant_system = format!("{SYSTEM_SID}:(OI)(CI)F");
        let grant_admins = format!("{ADMINS_SID}:(OI)(CI)F");
        super::run_cmd(
            "icacls",
            &[
                &dir,
                "/inheritance:r",
                "/grant:r",
                &grant_system,
                &grant_admins,
            ],
        )?;

        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )?;
        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: "RustyRMM Agent".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe.to_path_buf(),
            launch_arguments: vec!["--state-dir".into(), state_dir.into(), "service".into()],
            dependencies: vec![],
            account_name: None, // LocalSystem
            account_password: None,
        };
        let service =
            manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
        service.set_description("Keeps this Endpoint connected to its RustyRMM Server.")?;
        service.start::<&str>(&[])?;
        Ok(())
    }

    pub fn uninstall() -> anyhow::Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
        let service = match manager.open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()), // Not installed.
        };
        if service.query_status()?.current_state != ServiceState::Stopped {
            service.stop()?;
            for _ in 0..50 {
                if service.query_status()?.current_state == ServiceState::Stopped {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        service.delete()?;
        Ok(())
    }

    static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    /// Entry point when started by the Service Control Manager (`service` subcommand).
    pub fn run_as_service(state_dir: &Path) -> anyhow::Result<()> {
        STATE_DIR.get_or_init(|| state_dir.to_path_buf());
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
        Ok(())
    }

    fn service_main(_args: Vec<OsString>) {
        if let Err(e) = run_service() {
            eprintln!("service failed: {e:#}");
        }
    }

    fn run_service() -> anyhow::Result<()> {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let mut stop_tx = Some(stop_tx);
        let status = service_control_handler::register(SERVICE_NAME, move |event| match event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(tx) = stop_tx.take() {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;
        let set = |state: ServiceState| {
            status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: if state == ServiceState::Running {
                    ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
                } else {
                    ServiceControlAccept::empty()
                },
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
        };
        set(ServiceState::Running)?;
        let state_dir = STATE_DIR.get().expect("set before dispatch").clone();
        let result = tokio::runtime::Runtime::new()?.block_on(async {
            tokio::select! {
                r = crate::session::run(&state_dir) => r,
                _ = stop_rx => Ok(()),
            }
        });
        set(ServiceState::Stopped)?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_runs_the_installed_binary_with_the_state_dir() {
        let unit = systemd_unit(
            Path::new("/usr/local/bin/rustyrmm-agent"),
            Path::new("/var/lib/rustyrmm-agent"),
        );
        assert!(unit.contains(
            "ExecStart=\"/usr/local/bin/rustyrmm-agent\" --state-dir \"/var/lib/rustyrmm-agent\" run"
        ));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn launchd_plist_escapes_paths_and_keeps_the_daemon_alive() {
        let plist = launchd_plist(
            Path::new("/usr/local/bin/rustyrmm-agent"),
            Path::new("/Library/Application Support/R&D <x>"),
        );
        assert!(plist.contains("<string>com.rustyrmm.agent</string>"));
        assert!(plist.contains("<string>/Library/Application Support/R&amp;D &lt;x&gt;</string>"));
        assert!(plist.contains("<key>KeepAlive</key>\n    <true/>"));
        assert!(plist.contains("<string>run</string>"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_passes_plutil_lint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.plist");
        std::fs::write(
            &path,
            launchd_plist(
                Path::new("/usr/local/bin/rustyrmm-agent"),
                &dir.path().join("state"),
            ),
        )
        .unwrap();
        run_cmd("plutil", &["-lint", path.to_str().unwrap()]).unwrap();
    }
}
