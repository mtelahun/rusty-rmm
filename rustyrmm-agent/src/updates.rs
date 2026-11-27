//! Pending OS update counts. Each OS has its own tool; anything that fails or can't be
//! classified stays unknown (absent), never 0.
// Every OS's parser is compiled and unit-tested everywhere, but each OS calls only its own.
#![allow(dead_code)]

use std::collections::HashMap;
use std::time::Duration;

use rustyrmm_proto::common::v1::{PendingUpdate, PendingUpdates, UpdateStatus};
use tokio::process::Command;

/// Update checks can be slow (metadata refreshes); give up after this long.
const TIMEOUT: Duration = Duration::from_secs(300);

/// One run of the update commands gives both the counts and the per-Update list. A failed
/// check gives unknown counts and an unknown list (`known: false`), never an empty one.
pub async fn check() -> (UpdateStatus, PendingUpdates) {
    os_check().await.unwrap_or_default()
}

fn known(updates: Vec<PendingUpdate>, refreshed: Option<std::time::SystemTime>) -> PendingUpdates {
    PendingUpdates {
        known: true,
        updates,
        metadata_refreshed_at: refreshed.map(prost_types::Timestamp::from),
    }
}

/// Runs a command and returns (exit code, stdout), or None if it can't run or times out.
async fn output(program: &str, args: &[&str]) -> Option<(i32, String)> {
    let run = Command::new(program).args(args).kill_on_drop(true).output();
    let out = tokio::time::timeout(TIMEOUT, run).await.ok()?.ok()?;
    Some((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

#[cfg(target_os = "linux")]
async fn os_check() -> Option<(UpdateStatus, PendingUpdates)> {
    // Debian/Ubuntu: a simulated (-s) dist-upgrade lists what would be installed. Plain
    // `upgrade` keeps back updates that need new packages, such as kernel updates.
    if let Some((0, out)) = output("apt-get", &["-s", "-q", "dist-upgrade"]).await {
        // An unparsable line makes the list unknown, but the counts stay as they are.
        let pending = parse_apt_updates(&out).map_or_else(PendingUpdates::default, |u| {
            known(u, mtime("/var/lib/apt/lists"))
        });
        return Some((parse_apt(&out), pending));
    }
    // Fedora/RHEL: check-update exits 100 when updates exist.
    if let Some((code @ (0 | 100), out)) = output("dnf", &["-q", "check-update"]).await {
        let total = if code == 0 {
            0
        } else {
            count_dnf_packages(&out)
        };
        let sec = match output("dnf", &["-q", "updateinfo", "list", "--security"]).await {
            Some((0, sec)) => Some(sec),
            _ => None,
        };
        let security = sec.as_deref().map(count_dnf_advisory_packages);
        let list = if code == 0 {
            vec![]
        } else {
            parse_dnf_updates(&out, sec.as_deref())
        };
        // shortcut: dnf5 keeps no such stamp, so its refresh time stays unknown.
        let pending = known(list, mtime("/var/cache/dnf/last_makecache"));
        return Some((split(total, security), pending));
    }
    None
}

#[cfg(target_os = "macos")]
async fn os_check() -> Option<(UpdateStatus, PendingUpdates)> {
    // softwareupdate has no reliable security flag, so security stays unknown.
    let (_, out) = output("softwareupdate", &["--list"]).await?;
    let status = UpdateStatus {
        regular: Some(parse_softwareupdate(&out)),
        security: None,
    };
    Some((status, PendingUpdates::default()))
}

#[cfg(windows)]
async fn os_check() -> Option<(UpdateStatus, PendingUpdates)> {
    // Windows Update Agent via COM; the GUID is the "Security Updates" category.
    const SCRIPT: &str = "$r = (New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher().Search(\"IsInstalled=0 and IsHidden=0 and Type='Software'\"); \
        $sec = @($r.Updates | Where-Object { $_.Categories | Where-Object { $_.CategoryID -eq '0FA1201D-4330-4FA8-8AE9-B877473B6441' } }).Count; \
        \"$($r.Updates.Count) $sec\"";
    let (0, out) = output(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", SCRIPT],
    )
    .await?
    else {
        return None;
    };
    let mut nums = out.split_whitespace().map(str::parse::<u32>);
    let (Some(Ok(total)), Some(Ok(security))) = (nums.next(), nums.next()) else {
        return None;
    };
    Some((split(total, Some(security)), PendingUpdates::default()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn os_check() -> Option<(UpdateStatus, PendingUpdates)> {
    None
}

#[cfg(target_os = "linux")]
fn mtime(path: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Regular = everything that isn't a security update.
fn split(total: u32, security: Option<u32>) -> UpdateStatus {
    UpdateStatus {
        regular: Some(total.saturating_sub(security.unwrap_or(0))),
        security,
    }
}

/// `apt-get -s dist-upgrade`: one `Inst` line per package; the origin in brackets names the
/// suite, and `-security` suites carry security updates.
pub fn parse_apt(out: &str) -> UpdateStatus {
    let inst: Vec<&str> = out.lines().filter(|l| l.starts_with("Inst ")).collect();
    let security = inst.iter().filter(|l| l.contains("-security")).count() as u32;
    split(inst.len() as u32, Some(security))
}

/// `apt-get -s dist-upgrade` as one entry per `Inst` line: `Inst name [installed] (new suite
/// [arch])`, where `[installed]` is missing for a new package. None if a line doesn't parse.
pub fn parse_apt_updates(out: &str) -> Option<Vec<PendingUpdate>> {
    out.lines()
        .filter_map(|l| l.strip_prefix("Inst ").map(|rest| (l, rest)))
        .map(|(l, rest)| {
            let (name, rest) = rest.split_once(' ')?;
            let (installed, rest) = match rest.strip_prefix('[') {
                Some(r) => {
                    let (v, r) = r.split_once("] ")?;
                    (Some(v.to_string()), r)
                }
                None => (None, rest),
            };
            let new_version = rest.strip_prefix('(')?.split_whitespace().next()?;
            Some(PendingUpdate {
                install_key: name.to_string(),
                // apt output gives no title, so the title is the package name.
                title: name.to_string(),
                security: Some(l.contains("-security")),
                new_version: new_version.to_string(),
                installed_version: installed,
                ..Default::default()
            })
        })
        .collect()
}

/// `dnf -q check-update` rows `name.arch  version  repo`; stops at the "Obsoleting Packages"
/// section, which repeats packages.
fn dnf_rows(out: &str) -> impl Iterator<Item = [&str; 3]> {
    out.lines()
        .take_while(|l| !l.starts_with("Obsoleting"))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            let [pkg, version, repo] = cols[..] else {
                return None;
            };
            pkg.contains('.').then_some([pkg, version, repo])
        })
}

pub fn count_dnf_packages(out: &str) -> u32 {
    dnf_rows(out).count() as u32
}

/// The `check-update` rows as entries. `advisories` is the `updateinfo list --security`
/// output, or None if that query failed, which leaves security unknown. dnf output gives no
/// title, so the title is the package name.
pub fn parse_dnf_updates(out: &str, advisories: Option<&str>) -> Vec<PendingUpdate> {
    // Advisory rows name the package as `name-version.arch`; keep the first advisory per package.
    let mut advised: HashMap<&str, (&str, &str)> = HashMap::new();
    for cols in advisories
        .iter()
        .flat_map(|a| a.lines())
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
    {
        if let [id, severity, .., pkg] = cols[..] {
            advised.entry(pkg).or_insert((id, severity));
        }
    }
    dnf_rows(out)
        .map(|[key, version, _repo]| {
            let (name, arch) = key.rsplit_once('.').unwrap_or((key, ""));
            let found = advised.get(format!("{name}-{version}.{arch}").as_str());
            PendingUpdate {
                install_key: key.to_string(),
                title: name.to_string(),
                security: advisories.map(|_| found.is_some()),
                new_version: version.to_string(),
                severity: found.map(|(_, s)| s.split('/').next().unwrap_or(s).to_string()),
                advisory_id: found.map(|(id, _)| id.to_string()),
                ..Default::default()
            }
        })
        .collect()
}

/// `dnf -q updateinfo list --security`: `ADVISORY  severity/Sec.  package`; count packages.
pub fn count_dnf_advisory_packages(out: &str) -> u32 {
    let mut packages: Vec<&str> = out
        .lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.len() >= 3).then(|| cols[cols.len() - 1])
        })
        .collect();
    packages.sort_unstable();
    packages.dedup();
    packages.len() as u32
}

/// `softwareupdate --list`: one `* Label: …` line per available update.
pub fn parse_softwareupdate(out: &str) -> u32 {
    out.lines()
        .filter(|l| l.trim_start().starts_with("* Label:"))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_counts_security_separately() {
        let out = "NOTE: This is only a simulation!\n\
            Inst libssl3 [3.0.2-0ubuntu1.14] (3.0.2-0ubuntu1.15 Ubuntu:22.04/jammy-security [amd64])\n\
            Inst curl [7.81.0-1ubuntu1.15] (7.81.0-1ubuntu1.16 Ubuntu:22.04/jammy-updates [amd64])\n\
            Inst vim [2:8.2] (2:8.2.1 Ubuntu:22.04/jammy-updates [amd64])\n\
            Conf libssl3 (3.0.2-0ubuntu1.15 Ubuntu:22.04/jammy-security [amd64])\n";
        let s = parse_apt(out);
        assert_eq!((s.regular, s.security), (Some(2), Some(1)));
    }

    #[test]
    fn dnf_counts_packages_and_ignores_obsoletes() {
        let out = "\nkernel.x86_64   6.8.1-1.fc40   updates\n\
            openssl.x86_64  1:3.2.1-2.fc40 updates\n\
            Obsoleting Packages\n\
            foo.noarch      1.0-1.fc40     updates\n";
        assert_eq!(count_dnf_packages(out), 2);
        let sec = "FEDORA-2024-aa Important/Sec. openssl-1:3.2.1-2.fc40.x86_64\n\
            FEDORA-2024-bb Moderate/Sec.  openssl-1:3.2.1-2.fc40.x86_64\n";
        assert_eq!(count_dnf_advisory_packages(sec), 1);
        let s = split(2, Some(1));
        assert_eq!((s.regular, s.security), (Some(1), Some(1)));
    }

    #[test]
    fn softwareupdate_counts_labels() {
        let out = "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n\
            * Label: macOS Sonoma 14.5-23F79\n\tTitle: macOS Sonoma 14.5, Version: 14.5, Size: 1G, Recommended: YES, Action: restart,\n\
            * Label: Safari17.5-17.5\n\tTitle: Safari, Version: 17.5, Size: 1M, Recommended: YES,\n";
        assert_eq!(parse_softwareupdate(out), 2);
    }

    const APT: &str = "NOTE: This is only a simulation!\n\
        Inst libssl3 [3.0.2-0ubuntu1.14] (3.0.2-0ubuntu1.15 Ubuntu:22.04/jammy-security [amd64])\n\
        Inst curl [7.81.0-1ubuntu1.15] (7.81.0-1ubuntu1.16 Ubuntu:22.04/jammy-updates [amd64])\n\
        Inst newpkg (1.0 Ubuntu:22.04/jammy-updates [amd64])\n\
        Conf libssl3 (3.0.2-0ubuntu1.15 Ubuntu:22.04/jammy-security [amd64])\n";
    const DNF: &str = "\nkernel.x86_64   6.8.1-1.fc40   updates\n\
        openssl.x86_64  1:3.2.1-2.fc40 updates\n\
        Obsoleting Packages\n\
        foo.noarch      1.0-1.fc40     updates\n";
    const DNF_SEC: &str = "FEDORA-2024-aa Important/Sec. openssl-1:3.2.1-2.fc40.x86_64\n";

    #[test]
    fn apt_lists_each_update() {
        let u = parse_apt_updates(APT).unwrap();
        let got: Vec<_> = u
            .iter()
            .map(|u| {
                (
                    &*u.install_key,
                    &*u.new_version,
                    u.installed_version.as_deref(),
                    u.security,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "libssl3",
                    "3.0.2-0ubuntu1.15",
                    Some("3.0.2-0ubuntu1.14"),
                    Some(true)
                ),
                (
                    "curl",
                    "7.81.0-1ubuntu1.16",
                    Some("7.81.0-1ubuntu1.15"),
                    Some(false)
                ),
                ("newpkg", "1.0", None, Some(false)),
            ]
        );
        assert_eq!(u[0].title, "libssl3");
        assert!(parse_apt_updates("Inst broken\n").is_none());
    }

    #[test]
    fn dnf_lists_name_arch_with_security_from_advisories() {
        let u = parse_dnf_updates(DNF, Some(DNF_SEC));
        let got: Vec<_> = u
            .iter()
            .map(|u| (&*u.install_key, &*u.title, u.security))
            .collect();
        assert_eq!(
            got,
            [
                ("kernel.x86_64", "kernel", Some(false)),
                ("openssl.x86_64", "openssl", Some(true))
            ]
        );
        assert_eq!(u[1].new_version, "1:3.2.1-2.fc40");
        assert_eq!(u[1].severity.as_deref(), Some("Important"));
        assert_eq!(u[1].advisory_id.as_deref(), Some("FEDORA-2024-aa"));
    }

    #[test]
    fn dnf_security_is_unknown_when_the_advisory_query_failed() {
        let u = parse_dnf_updates(DNF, None);
        assert_eq!(u.len(), 2);
        assert!(u.iter().all(|u| u.security.is_none()));
    }

    #[test]
    fn failed_check_is_an_unknown_list_and_empty_check_a_known_empty_one() {
        use prost::Message;
        let failed = PendingUpdates::default();
        let empty = known(vec![], None);
        assert!(!failed.known && empty.known && empty.updates.is_empty());
        assert_ne!(failed.encode_to_vec(), empty.encode_to_vec());
    }
}
