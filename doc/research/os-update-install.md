# Listing and installing OS Updates unattended (apt, dnf, Windows)

Research for [#96](https://github.com/mtelahun/rusty-rmm/issues/96). The question: how does
the Agent, running as a service (root on Linux, LocalSystem on Windows), list each pending
Update in detail and install chosen Updates unattended? Today
`rustyrmm-agent/src/updates.rs` only counts them.

Sources are primary: man pages read on the research machine (Ubuntu) (apt 3.2.1ubuntu0.1, dpkg 1.23.7ubuntu1,
unattended-upgrades 2.12ubuntu9, systemd 259), upstream source for apt and
unattended-upgrades, the dnf 4 and dnf5 docs, Microsoft Learn, and the `windows` 0.62.2 crate
source. Each claim carries its source. Where no primary source says something, the doc says
so ("not documented") instead of guessing. The dnf pages were read through a summarizer, so
they are paraphrased with links. The facts this doc relies on (exit codes 100 and 200,
`-y`, `--json` fields, needs-restarting exit codes) were checked against the raw `.rst`
sources. Quoted text is verbatim from the source.

## Summary

- **apt.** `apt-get -s dist-upgrade` already gives name, current and new version, and origin
  (the `-security` suite marks security Updates). It gives no size, severity or
  reboot flag. Install unattended with
  `DEBIAN_FRONTEND=noninteractive apt-get -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold -o DPkg::Lock::Timeout=<s>`.
  apt has no security-only switch: choose the security Updates from the `Inst` lines and
  pass them to `install --only-upgrade`. Exit codes are per run (0 or 100), so get
  per-Update results from `APT::Status-Fd` and a `dpkg-query` check afterwards.
- **dnf.** dnf5 has `check-upgrade --json` and `advisory list --json`, which give name, arch,
  new EVR, repo, advisory type and severity. dnf 4 has no JSON, so parse its columns.
  Install with `dnf -y upgrade [--security | <pkgs>]`. In dnf 4 `check-update` exits 100
  when Updates exist, and a lock failure exits 200.
- **Windows.** Use the Windows Update Agent (WUA) COM API. Listing works through today's
  PowerShell route. Installing should call WUA in-process from Rust via the `windows` crate
  (feature `Win32_System_UpdateAgent`), which gives typed per-Update `ResultCode` and
  `HResult` and skips parsing PowerShell output. `IUpdate` carries KB IDs, `MsrcSeverity`,
  categories, sizes and `InstallationBehavior.RebootBehavior`. Under WSUS, WUA searches the
  WSUS server by default, so the Agent sees only what WSUS approved.
- **Reboot.** Debian/Ubuntu: `/var/run/reboot-required` (written by update-notifier and
  unattended-upgrades kernel hooks). dnf: `needs-restarting -r` (dnf 4) or
  `needs-restarting` (dnf5), where exit 1 means a reboot is needed. Windows:
  `ISystemInformation.RebootRequired`. Reboot with `shutdown -r +N` or `InitiateShutdownW`,
  and only after the Command's result is sent.
- **Biggest hazard.** The Agent's systemd unit uses the default `KillMode=control-group`, and
  a Command timeout kills the whole process group. So an Agent restart, or the 1 h cap,
  kills apt or dpkg partway and leaves packages half-configured. Run installs in their own
  transient unit (`systemd-run`) and always run `dpkg --configure -a` before the next run.
- **Duration.** No primary source gives install times. They need measuring (see open
  questions).

## 1. Listing

### apt (Debian/Ubuntu)

| Field | Where it comes from |
|---|---|
| name, current version, new version, origin/suite, arch | `apt-get -s dist-upgrade` `Inst` lines: `Inst <pkg> [<cur>] (<new> <origin>:<release>/<suite> [<arch>])`, for example `Inst libssl3 [3.0.2-0ubuntu1.14] (3.0.2-0ubuntu1.15 Ubuntu:22.04/jammy-security [amd64])`. In simulations, `Conf`/`Remv`/`Inst` stand for configure, remove and unpack [apt-get(8), `-s`] |
| security flag | the suite ends in `-security` (what `updates.rs` does now). unattended-upgrades uses the same origin matching (`${distro_id}:${distro_codename}-security` in `/etc/apt/apt.conf.d/50unattended-upgrades`) |
| download size | not in `Inst` lines. `apt-get --print-uris` prints "the path, the destination file name, the size and the expected MD5 hash" for each file [apt-get(8)], and `apt-cache show <pkg>=<ver>` has a `Size:` field |
| severity, CVE | not in apt metadata (Debian and Ubuntu publish them separately in their security trackers, not through apt) |
| needs reboot | not knowable before install. It is known only after install, from `/var/run/reboot-required` (section 5) |
| removals | `Remv` lines. `dist-upgrade` "may therefore remove some packages" [apt-get(8)] |

- Use `apt-get`, not `apt`: apt(8) says its CLI "is designed as an end-user tool and it may
  change behavior between versions" [apt(8), "Script usage"].
- `-s` runs without locks ("Locking will be disabled (Debug::NoLocking)") and as non-root may
  not read all the configuration [apt-get(8)]. The Agent runs as root, so the second point
  doesn't apply.
- **Metadata refresh.** `upgrade` needs "an update must be performed first so that apt-get
  knows that new versions of packages are available" [apt-get(8)]. `apt-get update` takes
  the lists lock. apt 3 adds `-U/--update` to run update before install, but notes "locks are
  acquired individually for each phase", which makes it "merely syntactic sugar for apt
  update && apt install" [apt-get(8), apt 3.x only]. Ubuntu's apt-daily timer refreshes the
  lists daily by default (`APT::Periodic::Update-Package-Lists "1"` on the research machine). The
  Agent could rely on that for listing and refresh only before installing.
- **Time.** A simulate run on the research machine took 2.3 s wall clock (measured). Refresh time
  depends on mirrors and the network, and no source documents it.

### dnf (Fedora, RHEL)

dnf 4 and dnf5 differ in places, and both are in use (which distro release ships which was
not checked against a primary source):

| | dnf 4 | dnf5 |
|---|---|---|
| list pending | `dnf check-update`: exit 100 = Updates listed, 0 = none, 1 = error [[dnf4 command ref](https://dnf.readthedocs.io/en/latest/command_ref.html)] | `dnf5 check-upgrade`: same 100/0 rule. `--json` gives `name`, `arch`, `evr`, `repository` (and `obsoletes`) [[check-upgrade.8](https://dnf5.readthedocs.io/en/latest/commands/check-upgrade.8.html)] |
| security / severity | `dnf updateinfo list --security`, columns: advisory ID, type/severity, NEVRA. `--sec-severity` filters [[dnf4 command ref](https://dnf.readthedocs.io/en/latest/command_ref.html)] | `dnf5 advisory list --updates --security --json` gives `name`, `type`, `severity`, `nevra`, `buildtime`. `--with-cve` adds `references`. `--advisory-severities=critical,important,...` filters [[advisory.8](https://dnf5.readthedocs.io/en/latest/commands/advisory.8.html)] |
| needs reboot (before install) | advisories can carry `reboot_suggested` (dnf5 needs-restarting uses it, see section 5). No list command exposes it directly in the docs read | same |
| size | not in `check-update` output. `dnf repoquery`/`info` show size (not checked further) | same |
| refresh | `metadata_expire` defaults to 48 h. `--refresh` marks metadata expired before the command. `makecache` downloads metadata ahead of time [[dnf4 conf ref](https://dnf.readthedocs.io/en/latest/conf_ref.html)] | `--refresh` forces a refresh. `-C/--cacheonly` uses only the cache [[dnf5.8](https://dnf5.readthedocs.io/en/latest/dnf5.8.html)] |

A listing therefore refreshes on its own when the cache is older than 48 h, so a listing
Command can take as long as a full metadata download.

### Windows (Windows Update Agent)

`IUpdateSearcher::Search(criteria)` with
`IsInstalled=0 and IsHidden=0 and Type='Software'` (what `updates.rs` sends now). Criteria
also include `CategoryIDs contains '<guid>'`, `RebootRequired=1`, `BrowseOnly=0/1` (optional
Updates), `AutoSelectOnWebSites` and `UpdateID` [[IUpdateSearcher::Search](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdatesearcher-search)].

Per-Update fields on `IUpdate` [[IUpdate](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nn-wuapi-iupdate)]:

| Field | Property |
|---|---|
| stable id | `Identity.UpdateID` + `Identity.RevisionNumber` (searchable by `UpdateID='…'`) |
| title, description | `Title`, `Description` |
| KB number | `KBArticleIDs` (collection) |
| security flag | `Categories` contains `0FA1201D-4330-4FA8-8AE9-B877473B6441` (Security Updates, used by `updates.rs` today) |
| severity | `MsrcSeverity`: Critical, Important, Moderate or Low (the value for non-security Updates is not documented) [[MsrcSeverity](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdate-get_msrcseverity)] |
| size | `MaxDownloadSize`, `MinDownloadSize` |
| reboot | `InstallationBehavior.RebootBehavior`: 0 never, 1 always, 2 can request [[InstallationRebootBehavior](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/ne-wuapi-installationrebootbehavior)] |
| needs a user | `InstallationBehavior.CanRequestUserInput` |
| other | `IsDownloaded`, `EulaAccepted`, `IsMandatory`, `Deadline`, `Type`, `SupersededUpdateIDs` |

`IUpdateSearcher.Online` sets "whether the UpdateSearcher goes online to search for updates"
[[Online](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdatesearcher-get_online)].
The page gives no default. The Microsoft sample never sets it and still finds new Updates, so
no separate refresh step appears to exist (inferred, verify on a VM).
Microsoft warns that scans can use "higher than normal memory"
[[Searching, Downloading, and Installing Updates](https://learn.microsoft.com/en-us/windows/win32/wua_sdk/searching--downloading--and-installing-updates)].
No doc gives scan duration. `Windows.Management.Update.WindowsUpdateManager` is a newer WinRT
API, introduced in build 10.0.23504 (Windows 11 era), for scanning and watching progress
[[WindowsUpdateManager](https://learn.microsoft.com/en-us/uwp/api/windows.management.update.windowsupdatemanager?view=winrt-26100)].
It has no install method like `IUpdateInstaller` and needs a newer Windows, so WUA remains the
API to use.

## 2. Installing

### apt

```
DEBIAN_FRONTEND=noninteractive apt-get -y -q \
  -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold \
  -o DPkg::Lock::Timeout=300 -o APT::Status-Fd=3 \
  dist-upgrade                         # all
  install --only-upgrade pkg1 pkg2     # named subset or security-only
```

- `-y`: "assume 'yes' as answer to all prompts and run non-interactively". It still aborts on
  "changing a held package, trying to install an unauthenticated package or removing an
  essential package" [apt-get(8)]. Don't add `--allow-change-held-packages` ("can potentially
  destroy your system").
- `DEBIAN_FRONTEND=noninteractive`: debconf "never interacts with you at all" and takes the
  default answers [[debconf(7)](https://manpages.debian.org/trixie/debconf-doc/debconf.7.en.html)].
- Conffile prompts come from dpkg, not debconf. `--force-confdef` takes the default action,
  and where there is none, `--force-confold` keeps the locally modified file [dpkg(1),
  `--force-conf*`]. Without them a modified conffile stops the run at a prompt.
- `--only-upgrade`: "Do not install new packages … only-upgrade will install upgrades for
  already installed packages only" [apt-get(8)]. `pkg=version` pins the version picked at
  listing time.
- **Security-only.** apt-get has no security filter. Two routes have sources:
  (a) run `unattended-upgrade`, which installs only from `Unattended-Upgrade::Allowed-Origins`
  (security by default) and handles conffile prompts itself [unattended-upgrade(8)]; or
  (b) select the `-security` `Inst` lines from the simulation and pass those names to
  `install --only-upgrade`. Route (b) keeps the Agent in control and reuses `parse_apt`.
- **Exit codes.** "apt-get returns zero on normal operation, decimal 100 on error"
  [apt-get(8)]. A whole run gets one code. Packages kept back are normal operation, not an
  error (inferred: the man page lists only these two codes).
- **Per-Update result.** `APT::Status-Fd=<fd>` writes machine-readable lines:
  `pmstatus:<pkg>:<pct>:<text>`, `pmerror:<deb>:<pct>:<error>`, and
  `pmconffile:<conffile>:…` (a conffile prompt is pending)
  [[progress-reporting.md](https://salsa.debian.org/apt-team/apt/-/raw/main/doc/progress-reporting.md)].
  Afterwards, `dpkg-query -W -f='${Status} ${Version}\n' <pkg>` confirms what is installed.
  This works even when the run was cut short.

### dnf

```
dnf -y upgrade                          # all
dnf -y upgrade --security               # security-only (dnf5 also: --advisory-severities=critical,important)
dnf -y upgrade pkg1 pkg2                # named subset
dnf -y upgrade --advisories=FEDORA-…    # by advisory (dnf 4 accepts --advisory= or --advisories=)
```

- `-y/--assumeyes`: "Automatically answer yes for all questions." [dnf4 command_ref.rst].
  Whether rpm can prompt about config files was not checked (rpm is expected to write
  `.rpmnew` files instead; unverified).
- `--skip-broken` (dnf 4) drops packages that cause depsolve problems.
  `--skip-unavailable` (dnf5) skips packages that can't be upgraded and upgrades the rest
  [[upgrade.8](https://dnf5.readthedocs.io/en/latest/commands/upgrade.8.html)]. `--minimal` /
  `upgrade-minimal` upgrades to the lowest version that fixes the advisories.
- **Exit codes.** dnf 4: 0 success, 1 "An error occurred, which was handled by dnf", 3
  unknown error, 100 Updates available (check-update), 200 "There was a problem with
  acquiring or releasing of locks" [dnf4 command_ref.rst, "Return values"]. dnf5: 0, 1,
  2 (bad arguments), and per-command codes [dnf5.8].
- **Per-Update result.** The exit code covers the whole run. Check each package afterwards
  with `rpm -q <pkg>` or `dnf history info last`. No primary source read here describes a
  machine-readable per-package result stream like apt's Status-Fd, or says whether an rpm
  transaction is all-or-nothing (open question).

### Windows

The flow Microsoft documents [[Searching, Downloading, and Installing Updates](https://learn.microsoft.com/en-us/windows/win32/wua_sdk/searching--downloading--and-installing-updates)]:

1. `UpdateSession` → `CreateUpdateSearcher().Search(criteria)`.
2. Build an `UpdateColl` of the chosen Updates (all of them; security ones by category; or a
   subset by `UpdateID`). Skip any with `CanRequestUserInput`. Call `AcceptEula()` where
   `EulaAccepted` is false (only if the Operator's policy allows it). Put an Update whose
   `InstallationBehavior.Impact = 2` (exclusive) in a batch on its own.
3. `CreateUpdateDownloader()`, `.Updates = coll`, `.Download()`, then keep those with
   `IsDownloaded`.
4. `CreateUpdateInstaller()`, `.Updates = coll`, `.Install()` → `IInstallationResult`.
   `ResultCode` is the overall result, and `GetUpdateResult(i).ResultCode` / `.HResult` give
   each Update's result: 2 succeeded, 3 succeeded with errors, 4 failed, 5 aborted
   [[OperationResultCode](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/ne-wuapi-operationresultcode)].
   The sample flags HRESULT `-2145116147` ("needed additional downloaded content; rerun").
   `IInstallationResult.RebootRequired` reports whether a reboot is needed.

- `Install` returns `WU_E_INSTALL_NOT_ALLOWED` while another install is running (check
  `IsBusy` first) and `WU_E_NO_UPDATE` for an empty collection
  [[IUpdateInstaller::Install](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdateinstaller-install)].
- `IUpdateInstaller2.ForceQuiet` forces Windows Installer to run silently. Updates that can't
  run silently fail with `WU_E_UH_DOESNOTSUPPORTACTION`
  [[ForceQuiet](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdateinstaller2-get_forcequiet)].
- **From a service.** Microsoft restricts *remote* callers: `CreateUpdateDownloader`,
  `CreateUpdateInstaller`, `AcceptEula` and `ISystemInformation` are unavailable remotely
  [[Using WUA From a Remote Computer](https://learn.microsoft.com/en-us/windows/win32/wua_sdk/using-wua-from-a-remote-computer)].
  No doc restricts a *local* LocalSystem caller. Local service use is "not stated; verify on a
  VM". Today's PowerShell listing already runs as LocalSystem.
- **Async.** `BeginSearch/BeginDownload/BeginInstall` have no guaranteed completion time, and
  Microsoft recommends the caller set its own timeout
  [[Guidelines for Asynchronous WUA Operations](https://learn.microsoft.com/en-us/windows/win32/wua_sdk/guidelines-for-asynchronous-wua-operations)].
  The synchronous calls on a dedicated thread (`tokio::task::spawn_blocking`) are simpler.

**COM from Rust or PowerShell.**

- `windows` 0.62.2 generates the whole WUA surface under feature `Win32_System_UpdateAgent`
  (plus `Win32_System_Com` for `CoCreateInstance`/`CoInitializeEx`). It contains `UpdateSession`
  and `SystemInformation` CLSIDs, `IUpdateSearcher::Search(&BSTR)`, `IUpdateInstaller::Install`,
  `IsBusy`, `RebootRequiredBeforeInstallation`, `IUpdateInstaller2::SetForceQuiet`,
  `IUpdate::KBArticleIDs`/`MsrcSeverity`, `ISystemInformation::RebootRequired`, and the
  `OperationResultCode` constants (read from the crate source,
  `src/Windows/Win32/System/UpdateAgent/mod.rs`; docs.rs does not render that module).
- In-process COM gives typed results and HRESULTs per Update, with no script text to keep in
  sync. The costs are `unsafe` calls, COM initialisation on a blocking thread, and a new
  Windows-only dependency (the Agent has only `windows-service` today).
- PowerShell needs nothing new and already works for counts, but every field has to go
  through text (or `ConvertTo-Json`). Installing that way leaves a long-running child
  process that the Command timeout can kill.
- PSWindowsUpdate is a third-party module, not built in. The old built-in
  `WindowsUpdateProvider` cmdlets (`Start-WUScan`, `Install-WUUpdates`) have no current
  Microsoft reference page. Microsoft Q&A threads report them gone from Windows 10 2004 on
  (community-reported, not official). Don't depend on either.

**WSUS and Group Policy.** With "Specify intranet Microsoft update service location" set,
"applications that use the Windows Update Agent APIs … normally operate against the specified
WSUS server". "Do not connect to any Windows Update Internet locations" blocks searches
against any other service
[[WSUS Group Policy settings](https://learn.microsoft.com/en-us/windows-server/administration/windows-server-update-services/deploy/4-configure-group-policy-settings-for-automatic-updates)].
`ServerSelection` `ssDefault` searches the default server, which is the managed (WSUS)
server when one is configured [[ServerSelection](https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/wuapicommon/ne-wuapicommon-serverselection.md)].
So on a WSUS-managed Endpoint the Agent lists and installs only what WSUS offers. Don't
override `ServerSelection`. Automatic Updates (AU) settings such as option 4 "auto download
and schedule the install" can install and reboot on their own schedule alongside the Agent.

## 3. Hazards and recovery

| Hazard | apt | dnf | Windows |
|---|---|---|---|
| another manager running | apt takes `/var/lib/dpkg/lock-frontend` then `/var/lib/dpkg/lock`. `DPkg::Lock::Timeout` (default 0 = fail at once; seconds; negative = no limit) makes apt wait, printing "Waiting for cache lock" [apt source `apt-pkg/deb/debsystem.cc`, `GetLockMaybeWait`]. unattended-upgrades holds the same lock while it runs [`/usr/bin/unattended-upgrade`] | dnf 4 `exit_on_lock` defaults to False (wait). A lock failure exits 200 [[dnf4 conf ref](https://dnf.readthedocs.io/en/latest/conf_ref.html)]. dnf5 documents no waiting behaviour, only `--skip-file-locks` [dnf5.8] | `IsBusy`, `WU_E_INSTALL_NOT_ALLOWED` |
| held / excluded | `apt-mark hold` "is used to mark a package as held back, which will prevent the package from being automatically installed, upgraded or removed" [apt-mark(8)]. `-y` aborts instead of changing holds. `apt-mark showhold` lists them | `excludepkgs` (repo option), `-x/--exclude` [dnf4 conf ref; dnf5.8]. A `versionlock` plugin exists [dnf4 command_ref.rst] | hidden Updates (`IsHidden`) are left out by the default criteria |
| interrupted install | dpkg leaves packages unpacked but not configured, and its journal under `/var/lib/dpkg/updates/` dirty (unattended-upgrades checks this, `is_dpkg_journal_dirty`). Recover with `dpkg --configure -a`, then `apt-get -f install`. `dpkg --audit` lists half-installed packages [dpkg(1)] | not documented in the pages read. dnf 4 has `remove --duplicates` to "Remove older versions of duplicated packages" [dnf4 command_ref.rst]. Whether an interrupted run leaves duplicates is an open question | not documented in the pages read (open question) |
| partial failure | one `pmerror` per failed package. The run exits 100 | the whole transaction fails, or `--skip-broken` / `--skip-unavailable` drop the failing packages | per-Update `ResultCode` 3/4 inside an overall 3 |
| service restarts during install | `needrestart` (if installed) has apt and dpkg hooks that call it "after any package installation/upgrades" [[needrestart README](https://raw.githubusercontent.com/liske/needrestart/master/README.md)]. Restart mode is "(l)ist only, (i)nteractive or (a)utomatically". Interactive mode run non-interactively "will fallback to list only mode" [[needrestart.conf](https://raw.githubusercontent.com/liske/needrestart/master/ex/needrestart.conf)]. In automatic mode it may restart the Agent itself (e.g. after a libc Update; inferred) | dnf doesn't restart services | n/a |

**Agent-specific.** The Agent's systemd unit (`rustyrmm-agent/src/service.rs`) sets no
`KillMode`, so it gets `control-group`: on stop, "all remaining processes in the control group
of this unit will be killed" [systemd.kill(5)]. A Command timeout also kills its whole
process group (`commands.rs`, `process_group(0)`). Either one can kill dpkg partway. Mitigation:
start the install as its own transient unit (`systemd-run --unit=rustyrmm-update-<id> …`
[systemd-run(1)]) and poll it. Before every listing or install, check for a dirty dpkg state
and run `dpkg --configure -a`. On Windows the install runs inside the Windows Update service,
so killing the caller probably doesn't stop it. This is not documented.

## 4. Duration against the 1 h Command cap

The Server caps a Command at 3600 s (`MAX_COMMAND_TIMEOUT_SECS`, `operator_service.rs`).
No primary source gives typical or worst-case install times for apt, dnf or Windows. What is
documented or measured:

- apt simulate: 2.3 s here (measured). `apt-get update` and download times depend on the
  mirror.
- dnf metadata expires after 48 h, so a listing may first download all metadata.
- dnf-automatic's systemd timers add a random delay of up to 1 h before starting
  [[dnf-automatic](https://dnf.readthedocs.io/en/latest/automatic.html)].
- WUA async operations have no guaranteed completion time (above).

Because the 1 h cap is not known to be enough, and killing an install partway is the
worst outcome (section 3), installs should not be bounded by the Command cap. Run them
detached (transient unit on Linux, WUA on its own thread on Windows) and report when they
finish.

## 5. Reboot

| OS | Detect | Source |
|---|---|---|
| Debian/Ubuntu | `/var/run/reboot-required` exists. `/var/run/reboot-required.pkgs` lists the packages. Written by `/etc/kernel/postinst.d/update-notifier` (update-notifier-common), `/etc/kernel/postinst.d/unattended-upgrades`, and package postinsts that call `/usr/share/update-notifier/notify-reboot-required` (e.g. libc6, dbus here). Whether anything else writes it on a Debian Endpoint without these packages was not checked | files on the research machine; `reboot_required()` in `/usr/bin/unattended-upgrade` checks the same path |
| dnf 4 | `dnf needs-restarting -r`: exit 1 = reboot required, 0 = not | [[needs_restarting](https://dnf-plugins-core.readthedocs.io/en/latest/needs_restarting.html)] |
| dnf5 | `dnf5 needs-restarting` (plugin, `dnf5-plugins`): exit 1 = reboot recommended. Checks the kernel, systemd, `reboot_suggested` advisories, and `suggest-reboot.d` drop-ins since boot. As root it also writes `/run/reboot-required`. `-r` is a no-op kept for compatibility | [[needs_restarting.8](https://dnf5.readthedocs.io/en/latest/dnf5_plugins/needs_restarting.8.html)] |
| Windows | `ISystemInformation.RebootRequired` (CLSID `SystemInformation`) or `IInstallationResult.RebootRequired`. Before installing: `IUpdateInstaller.RebootRequiredBeforeInstallation`. The WUA SDK does not mention the `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired` registry key. Azure Update Manager's docs list it as the key that "Tracks reboot status", so it is a fallback, but the API is the documented route | [[RebootRequired](https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-isysteminformation-get_rebootrequired)], [[Azure Update Manager](https://learn.microsoft.com/en-us/azure/update-manager/configure-wu-agent)] |

**Rebooting gracefully.** Linux: `shutdown -r +<min> "<msg>"` (unattended-upgrades runs
`/sbin/shutdown -r <when>`, and dnf-automatic's default `reboot_command` is
`shutdown -r +5 '…'`). Windows: `InitiateShutdownW(NULL, msg, grace, SHUTDOWN_RESTART |
SHUTDOWN_FORCE_OTHERS, reason)`. It needs `SE_SHUTDOWN_NAME` enabled in the token. A grace
period above 0 needs `SHUTDOWN_FORCE_SELF`. Pass a real reason code (0 logs an unplanned
shutdown). It returns once the request is accepted, and "the shutdown can still be aborted"
[[InitiateShutdownW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-initiateshutdownw)].
`SHUTDOWN_INSTALL_UPDATES` installs pending Updates during shutdown.

**Caveat.** A reboot issued inside a Command can take the Agent down before it sends the
result. Send the result first, then reboot with a delay (`+1` minute or a grace period), or
make the reboot its own Command. Report the outcome on reconnect from the new boot time.

## Open questions

- Real install durations on the target distros and Windows versions: measure on VMs (a
  Windows cumulative Update, a Fedora kernel Update, an Ubuntu point release).
- Does WUA `Install` behave the same in-process under LocalSystem as from an elevated
  console? Verify on a VM.
- Does killing a PowerShell/WUA caller cancel the install or leave it running in wuauserv?
- How do dnf5 locks behave with two dnf5 processes running at once? The docs read don't say.
- How do dnf and Windows recover from an install interrupted partway (power loss, killed
  process)? Is an rpm transaction all-or-nothing? The pages read don't say.
- Is `needrestart` installed by default on the Ubuntu Server versions in scope, and in which
  mode? Should the Agent set `$nrconf{restart}` to list-only, or exclude its own unit?
