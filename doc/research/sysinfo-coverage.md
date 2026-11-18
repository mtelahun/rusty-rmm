# sysinfo coverage of Endpoint Information and Monitoring

Ticket: #4. Question: does `sysinfo` (or an alternative) supply every field in `doc/feature-list.md` (Endpoint Information, Monitoring) on Windows, Linux and macOS, which need OS-specific code, and how are pending regular vs security OS updates counted?

Method: `sysinfo` 0.40.4 (latest stable on crates.io at time of writing; MSRV 1.95) source and API docs were read directly from the crate tarball. Vendor docs were fetched for update detection and virtualization. Claims not verified against a primary source are marked **[unverified]**. Nothing was run on a real Windows or macOS host; coverage is read from source, not tested.

## Summary

- `sysinfo` covers hostname, OS name/version, memory, CPU brand/vendor and counts, per-disk space, per-disk read/write counters, per-interface IPs and byte counters, and per-process PID/name/PPID/status/memory/CPU/disk/cmdline on all three OSes.
- It does NOT cover: timezone, OS virtualization system/role, CPU family/model as separate numeric fields, per-process network usage, and any update status. Those need OS-specific code or other crates.
- "io usage" (disk) and "utilization" (interface) are not percentages in sysinfo; both are derived by the Agent from byte counters over a refresh interval (and, for interface utilization, link speed).
- Proto notes: `MemInfo.total` is a `string` while `used` is `uint64` (likely a typo); `Disk.io_usage` and `NetInterface.utilization` need defined semantics; `UpdateStatus` cannot express "unknown" (needed on macOS).
- Update counts: no single API. Windows has the Windows Update Agent (WUA) COM API with a SecurityUpdates classification; dnf has `updateinfo` with `--security`; apt itself does not classify security updates; macOS `softwareupdate --list` is text-only and has no reliable security flag.

## Field-by-field coverage

Y = supplied by sysinfo; X = needs OS-specific code or another crate.

### Endpoint Information

| Field (feature-list) | sysinfo API | Linux | Windows | macOS |
|---|---|---|---|---|
| id/uuid | `Product::uuid()` (SMBIOS system UUID, returns `Result`) | Y (`/sys/devices/virtual/dmi/id/product_uuid`, normally root-readable only) | Y (SMBIOS) | Y |
| hostname | `System::host_name()` | Y | Y | Y |
| Timezone | none | X | X | X |
| OS full name | `System::long_os_version()`, `System::name()` | Y | Y | Y |
| OS family | `System::distribution_id()`, `distribution_id_like()`; `std::env::consts::FAMILY/OS` for coarse family | Y | Y (coarse) | Y (coarse) |
| OS version | `System::os_version()`, `kernel_version()` | Y | Y | Y |
| OS virtualization system | none | X | X | X |
| OS virtualization role | none | X | X | X |
| CPU family | `Cpu::vendor_id()` and `Cpu::brand()` only; no numeric family | X (numeric) | X (numeric) | X (numeric) |
| CPU model | `Cpu::brand()` (marketing string) | Y (brand string) | Y (brand string) | Y (brand string) |
| CPU core count | `System::physical_core_count()` | Y | Y | Y |
| CPU thread count | `System::cpus().len()` | Y | Y | Y |
| Memory total / usage | `total_memory()`, `used_memory()`, `available_memory()` | Y | Y | Y |
| Per disk total / usage | `Disk::total_space()`, `available_space()` (used = total - available) | Y | Y | Y |
| Per disk io usage | `Disk::usage()` -> `DiskUsage` bytes read/written (total and since last refresh) | Y (`/proc/diskstats`) | Y (`IOCTL_DISK_PERFORMANCE`) | Y (disk counters in src/unix/apple/disk.rs) |
| IP addresses | `Network::ip_networks()` per interface | Y | Y | Y |
| Update status (normal / security) | none | X | X | X |
| client (Agent) version | n/a, `env!("CARGO_PKG_VERSION")` | Y | Y | Y |

Notes:
- `Disks` skips tmpfs and network filesystems on Linux unless the `linux-tmpfs` / `linux-netdevs` features are on; listing a hung NFS/CIFS mount can block (docs in src/common/disk.rs).
- `Cpu` exposes only `name`, `vendor_id`, `brand`, `frequency`, `usage` (src/common/system.rs).
- `Product::uuid()` can be absent, duplicated on cloned VMs, or unreadable without privileges. The proto already keeps `system_uuid` separate from `RustyRmmId`, which is the right shape.

### Monitoring (real-time)

| Field | sysinfo API | Linux | Windows | macOS |
|---|---|---|---|---|
| CPU usage | `System::global_cpu_usage()`, `Cpu::usage()` (needs two refreshes) | Y | Y (PDH counters) | Y |
| Memory usage | `used_memory()` | Y | Y | Y |
| Network usage (per interface) | `Network::received()/transmitted()` (delta since refresh), `total_*` | Y | Y | Y |
| Link speed (for utilization %) | `transmit_link_speed()`, `receive_link_speed()` -> `Option<u64>` bits/s | Y (sysfs `speed`; None if down or no driver value) | Y (`TransmitLinkSpeed`/`ReceiveLinkSpeed`) | Y (one value for both directions) |
| Process PID | `Pid` key of `System::processes()` | Y | Y | Y |
| Process name | `Process::name()` | Y | Y | Y |
| PPID | `Process::parent()` | Y | Y | Y |
| State | `Process::status()` -> `ProcessStatus` | Y | Y (coarser) **[unverified]** | Y |
| Memory usage | `Process::memory()` | Y | Y | Y |
| CPU usage | `Process::cpu_usage()` | Y | Y | Y |
| Disk usage | `Process::disk_usage()` | Y | Y | Y (`proc_pid_rusage`: `ri_diskio_bytesread/byteswritten`) |
| Net usage (per process) | none | X | X | X |
| Command line | `Process::cmd()` | Y | Y (may be empty for protected processes) **[unverified]** | Y |

Caveats:
- Reading other users' or protected processes can fail without elevation; the Agent normally runs as root/SYSTEM, which mitigates this. **[unverified per OS]**
- sysinfo has no per-process network method (src/common/system.rs). Doing it natively is OS-specific and heavy (Windows ETW kernel-network provider, Linux eBPF/netlink accounting, macOS NetworkStatistics/nettop) **[unverified]**. Recommendation: drop from v1 or mark best-effort per OS.

## Derived fields (decide semantics before finalizing the proto)

- Disk `io_usage` (`float`): sysinfo gives bytes read/written since last refresh, not busy time. Recommend replacing with read and write bytes/sec.
- Interface `utilization` (`float`): derive as `(rx+tx bytes / interval) * 8 / link_speed` when link speed is `Some`; `None` (down, virtual NICs, drivers without speed) must map to "unknown", not 0. `capacity` comes from link speed.
- `MemInfo.total` is `string` in `rustyrmm-endpoint.proto` while sysinfo returns `u64` bytes; fix to `uint64`.
- Disk `used` = `total_space - available_space`; on Unix `available_space` is what unprivileged users can use, so used includes root-reserved blocks.

## Fields needing OS-specific code

### Timezone
No sysinfo API. Options: the `iana-time-zone` crate (IANA name on all three OSes) **[unverified, not read]**; or direct: Linux/macOS `/etc/localtime` symlink target; Windows `GetDynamicTimeZoneInformation` (Windows names, needs mapping to IANA). WMI `Win32_ComputerSystem.CurrentTimeZone` is only a UTC offset in minutes (Microsoft docs), not a zone identity.

### OS virtualization system and role

| | System (what hypervisor/container) | Role |
|---|---|---|
| Linux | `systemd-detect-virt` prints an id. VMs: `qemu, kvm, amazon, zvm, vmware, microsoft, oracle, powervm, xen, bochs, uml, parallels, bhyve, qnx, acrn, apple, sre, google`; containers: `openvz, lxc, lxc-libvirt, systemd-nspawn, docker, podman, rkt, wsl, proot, pouch`. `--vm`/`--container` filter; non-zero exit when none (freedesktop man page). Without systemd: DMI vendor/product, already exposed by `Product::vendor_name()`/`name()`, plus CPUID hypervisor leaf. | guest (VM or container) vs bare metal; from `--vm`/`--container` |
| Windows | `Win32_ComputerSystem.HypervisorPresent` (bool, Windows 8 / Server 2012+) plus `Manufacturer`/`Model` from SMBIOS (Microsoft docs). `HypervisorPresent` is also true on a Hyper-V host (root partition) so guest vs host needs extra checks **[unverified]**. | guest vs Hyper-V host needs extra work **[unverified]** |
| macOS | `sysctl kern.hv_vmm_present` reportedly 1 in a VM guest **[unverified: not found in the Apple page checked; confirm with `man sysctl`/xnu source on a Mac]**; model string via `Product::name()` | guest vs not |

Cross-platform x86 fallback: `raw-cpuid` `CpuId::get_hypervisor_info()` (leaf 0x4000_00xx, requires a virtual CPU), `get_vendor_info()`, `get_feature_info()`, `get_processor_brand_string()` (docs.rs). x86/x86_64 only, so useless on Apple Silicon and ARM.

### CPU family and model (numeric)
sysinfo gives brand string and vendor id only. Numeric family/model: `raw-cpuid` on x86 (`get_feature_info()`; confirm accessor names); Linux `/proc/cpuinfo`; macOS `sysctl machdep.cpu.*`; Windows `Win32_Processor.Family` / registry `HARDWARE\DESCRIPTION\System\CentralProcessor\0` **[the last three unverified]**. Proposal: `model` = brand string (covered); `family` = vendor id + `System::cpu_arch()` unless a numeric family is truly required.

## Update status: pending regular vs security

All of these need privilege and are slow (seconds to minutes, may hit the network). Run on a schedule or on demand and cache; do not run on the monitoring tick.

### Windows
Windows Update Agent COM API, `IUpdateSearcher::Search(criteria)`. Default criteria are `IsInstalled = 0 and IsHidden = 0`; `CategoryIDs contains '<guid>'` filters by classification (Microsoft docs). Classification GUIDs (Microsoft WSUS doc): SecurityUpdates `0FA1201D-4330-4FA8-8AE9-B877473B6441`, CriticalUpdates `E6CF1350-C01B-414D-A61F-263D14D133B4`, Updates `CD5FFD1E-E932-4E3A-BF74-18BF0B1BBD83`, DefinitionUpdates `E0789628-CE08-4437-BE74-2495B842F43B`.
- Security: `IsInstalled=0 and IsHidden=0 and CategoryIDs contains '0FA1201D-...'`.
- Regular: pending set minus the security set, de-duplicated by `UpdateIdentity.UpdateID` (an update can have several categories). Decide whether Definition updates (Defender signatures) count; recommend excluding them.
- Rust: `windows` crate COM bindings for `wuapi` **[unverified]**, or spawn PowerShell with `Microsoft.Update.Session` (simpler, slower). Endpoints using WSUS see only what the WSUS server approves (`ServerSelection`).

### Debian/Ubuntu (apt)
`apt-get -s upgrade` simulates; "Inst" lines are the unpack operations for installs/upgrades; `upgrade` never removes packages, `dist-upgrade` may (Debian apt-get man page). The man page has no notion of security updates, so apt does not classify them. Security can be inferred from the candidate version's origin/suite (`*-security`; simulate output shows the origin in parentheses) **[unverified format]**. Ubuntu ships `/usr/lib/update-notifier/apt-check` printing `regular;security` **[unverified, not fetched]**. Alternative: libapt-pkg or the `rust-apt` crate **[unverified]**. Indexes must be refreshed first (`apt-get update`) or counts are stale.

### RHEL/Fedora (dnf)
`dnf check-update` exits 100 when updates exist, 0 when none, 1 on error. `dnf updateinfo summary` counts advisories by type with security by severity; `--security` restricts to security advisories (dnf command reference). Regular = `check-update` packages minus `--security check-update` packages. `updateinfo` counts advisories, not packages; use package lists for counts. Distros without updateinfo metadata (e.g. some derivatives) give no security data. zypper/pacman not researched.

### macOS
`softwareupdate --list` (`-l`) lists available updates; it excludes "security data" (config data) updates unless `--include-config-data` is given; some entries are tagged `[recommended]`; no documented machine-readable output (ss64 summary of the man page, a secondary source; confirm with `man softwareupdate` on a Mac). There is no authoritative security flag, so a security count is not reliably obtainable. Recommendation: `regular` = number of listed updates, `security` = unknown. `UpdateStatus` should become `optional uint32` fields (or carry a `supported` flag) so unknown is representable.

## Alternatives considered

| Crate / tool | Role | Verdict |
|---|---|---|
| `sysinfo` | base for everything marked Y | use |
| `raw-cpuid` | x86 hypervisor leaf, numeric family/model | add, x86 only |
| `windows` (windows-rs) | WUA COM, WMI, timezone | needed on Windows for updates/virt |
| `iana-time-zone` | timezone | candidate **[unverified]** |
| `wmi` crate | WMI from Rust | candidate **[unverified]** |
| `systemd-detect-virt` | Linux virtualization | spawn; fall back to DMI |

Other inventory crates (`heim`, `battery`) were not evaluated.

## Verdict

- Fully covered by sysinfo on all three OSes: hostname, id/uuid (privilege caveat), OS name/version/family, CPU model/cores/threads, memory, disk size/usage/IO counters, IPs, interface counters and link speed, CPU/memory real-time, process list (PID, name, PPID, state, memory, cpu, disk, cmdline).
- Derivation only: disk io usage, interface utilization.
- OS-specific code needed: timezone, virtualization system/role, numeric CPU family, update status (all OSes), per-process net usage (all OSes, recommend deferring).
- Weakest: macOS security update count, Windows host-vs-guest role, per-process net usage.

## Sources

- sysinfo 0.40.4: https://crates.io/crates/sysinfo/0.40.4 and https://docs.rs/sysinfo/0.40.4/ (source read: src/common/{system,disk,network}.rs, src/unix/linux/{disk,network,product}.rs, src/unix/apple/{disk,network,macos/process}.rs, src/windows/{disk,network,process,cpu,product}.rs)
- IUpdateSearcher::Search: https://learn.microsoft.com/en-us/windows/win32/api/wuapi/nf-wuapi-iupdatesearcher-search
- WSUS classification GUIDs: https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ff357803(v=vs.85)
- Win32_ComputerSystem: https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-computersystem
- dnf command reference: https://dnf.readthedocs.io/en/latest/command_ref.html
- apt-get(8): https://manpages.debian.org/trixie/apt/apt-get.8.en.html
- systemd-detect-virt: https://www.freedesktop.org/software/systemd/man/latest/systemd-detect-virt.html
- raw-cpuid CpuId: https://docs.rs/raw-cpuid/latest/raw_cpuid/struct.CpuId.html
- softwareupdate (secondary): https://ss64.com/mac/softwareupdate.html
