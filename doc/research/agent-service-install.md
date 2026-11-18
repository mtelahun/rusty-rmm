# Running the Agent as an OS service and installing it via cargo-dist

Issue: #5. Date: 2026-11-18.

Question: how should the Agent run as a background service on Windows, Linux and macOS? Can dist installers (msi, shell, powershell, homebrew) register and start a service? How does a Pairing link's Environment token reach the install?

## Short answer

- Each OS has a native service manager and the Agent needs a small per-OS integration. Only Windows needs Agent code (the Agent must speak the SCM protocol); systemd and launchd run a plain foreground process.
- dist installers only place binaries. None of the documented installers registers or starts a service. Registration has to be a separate step: an `install-service` subcommand run by the Operator or by the installer, or (msi only) a hand-edited WiX template.
- Pairing token: do not bake it into the installer artifact. Pass it at first run, for example `rustyrmm-agent pair <token>`, which stores it in a root-only location and then installs/starts the service.

## Windows: windows-service crate

Source: docs.rs/windows-service 0.8.1 (crate docs and `ServiceManager` page).

- Entry: `define_windows_service!` generates the FFI entry; `main` calls `service_dispatcher::start("name", ffi_service_main)`, which blocks until the service stops.
- Control: `service_control_handler::register` returns a status handle; the handler must accept `Interrogate` and typically `Stop`.
- Status: the service starts in `StartPending` and must report `Running`; long starts report `StartPending` with `checkpoint` and `wait_hint`, otherwise the system may kill it.
- Install (in code): `ServiceManager::local_computer(None, CREATE_SERVICE)` then `create_service(&ServiceInfo{ name, display_name, service_type: OWN_PROCESS, start_type, error_control, executable_path, launch_arguments, dependencies, account_name: None (= LocalSystem), .. }, access)`, then `set_description`. The docs example shows `ServiceStartType::OnDemand`; the auto-start variant was not shown on the pages read (verify in the `ServiceStartType` docs). `open_service`, `update_failure_actions` and `get_failure_actions` exist for restart-on-crash policy.
- Consequence: the Agent binary needs a service mode (dispatcher) distinct from console mode, plus install/uninstall subcommands using the code above.

## Linux: systemd

Source: systemd.service(5), systemd.exec(5) on freedesktop.org.

- Unit: `[Service] ExecStart=<absolute path> run`, `Restart=on-failure` (the man page recommends it for long-running services), `[Install] WantedBy=multi-user.target`. `Type=simple` needs no Agent code; `Type=notify` requires the Agent to send `READY=1` via sd_notify.
- State: `StateDirectory=rustyrmm-agent` creates `/var/lib/rustyrmm-agent` (exported as `$STATE_DIRECTORY`, kept when the unit stops). `DynamicUser=` implies `ProtectSystem=strict`; a remote-management Agent probably needs root, so it is likely unsuitable (design judgement, not from the docs).
- Secrets: `EnvironmentFile=` (absolute path, UTF-8, `KEY=value` lines; a missing file fails the start unless prefixed `-`), or `LoadCredential=ID:PATH`, which exposes the file read-only under `$CREDENTIALS_DIRECTORY` to the unit's user and root. Credentials suit a token better.
- Install means writing the unit to `/etc/systemd/system/`, then `systemctl daemon-reload` and `systemctl enable --now` (standard usage; not re-verified in this pass).

## macOS: launchd

Source: launchd.plist(5), read via the manpagez mirror of the man page. Apple's archive page did not load, so recheck against Apple docs.

- Keys: `Label` (required; file conventionally `<Label>.plist`), `ProgramArguments` (execvp argv), `RunAtLoad`, `KeepAlive` (bool or conditions dict), `EnvironmentVariables`, `UserName` (only honored when launchd runs as root), `StandardOutPath`.
- Location: `/Library/LaunchDaemons` for system-wide daemons (the Agent's case: runs at boot with no logged-in user); `/Library/LaunchAgents` and `~/Library/LaunchAgents` are per-user.
- Daemon plist ownership rules and `launchctl bootstrap` syntax were not verified.
- Do not put the token in `EnvironmentVariables`; read it from a root-only file instead.

## What dist can and cannot do

Source: dist book (axodotdev.github.io/cargo-dist/book): installers, installers/usage, installers/msi, installers/homebrew, reference/config.

Version: the repo pins `cargo-dist-version = "0.4.2"` in the root `Cargo.toml`; the latest release is v0.34.0 (2026-11-02, GitHub releases API). The tool is now named `dist`. Several options below postdate 0.4.2 (`bin-aliases` since 0.14.0, `{APP}_DOWNLOAD_URL` since 0.30.0), so upgrading is a prerequisite for relying on them.

- shell, powershell and homebrew are "fetching" installers: they download the platform archive and install binaries. The docs describe no custom post-install code, service blocks or installer parameters. Config covers only `install-path`, `install-success-msg`, `bin-aliases` and env vars (`{APP}_INSTALL_DIR`, `INSTALLER_NO_MODIFY_PATH`, download/token/proxy vars). Only `-v`/`-q` flags are documented, so there is no documented way to pass the token into these installers.
- msi is a "bundling" WiX v3 installer. `wix/main.wxs` is managed by dist; with `allow-dirty = ["msi"]` it can be hand-edited, which in principle allows a WiX service install or custom action (dist docs do not cover this; check WiX docs). Default install dir is `%ProgramFiles%\{app}`. Passing the token as an msi property is WiX/msiexec behavior, not documented by dist.
- homebrew: docs mention only formula name, `bin-aliases` and metadata. Service blocks and post-install hooks are not documented as customizable.
- Hooks that exist are CI-level only: `github-build-setup`, `publish-jobs`, `local-artifacts-jobs`. None run on the Endpoint.

## Recommendation

1. Ship one binary with subcommands: `run` (foreground; also the service entry, with a Windows dispatcher mode), `pair <token>`, `install-service`, `uninstall-service`.
2. `install-service` writes the OS-native definition (SCM entry via `windows-service`; systemd unit; launchd plist in `/Library/LaunchDaemons`) and starts it. It needs root/Administrator.
3. Use dist as-is to distribute binaries. The Pairing link shows a one-liner such as `curl ... | sh && sudo rustyrmm-agent pair <token> --install-service` (and a PowerShell equivalent). The Environment token appears only in the displayed command, not in the downloaded artifact.
4. Token handling: pairing exchanges the Pairing token for a long-lived Agent credential stored root-only (`StateDirectory`/`LoadCredential` on Linux, ProgramData ACL on Windows). Do not leave the token in service definitions or env.
5. Later option: hand-edited msi (`allow-dirty`) for silent enterprise install with a token property; evaluate separately.

## Open items

- Confirm the auto-start `ServiceStartType` variant and failure-action API in windows-service docs.
- Confirm launchd ownership rules and `launchctl bootstrap` syntax in Apple docs.
- Upgrade dist from 0.4.2 and re-check installer behavior listed above.
- CONTEXT.md is not present on this branch, so domain terms follow the ticket.
