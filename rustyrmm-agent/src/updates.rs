//! Pending OS update counts. Each OS has its own tool; anything that fails or can't be
//! classified stays unknown (absent), never 0.
// Every OS's parser is compiled and unit-tested everywhere, but each OS calls only its own.
#![allow(dead_code)]

use std::time::Duration;

use rustyrmm_proto::common::v1::UpdateStatus;
use tokio::process::Command;

/// Update checks can be slow (metadata refreshes); give up after this long.
const TIMEOUT: Duration = Duration::from_secs(300);

pub async fn check() -> UpdateStatus {
    os_check().await.unwrap_or_default()
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
async fn os_check() -> Option<UpdateStatus> {
    // Debian/Ubuntu: a simulated upgrade lists what would be installed.
    if let Some((0, out)) = output("apt-get", &["-s", "-q", "upgrade"]).await {
        return Some(parse_apt(&out));
    }
    // Fedora/RHEL: check-update exits 100 when updates exist.
    if let Some((code @ (0 | 100), out)) = output("dnf", &["-q", "check-update"]).await {
        let total = if code == 0 {
            0
        } else {
            count_dnf_packages(&out)
        };
        let security = match output("dnf", &["-q", "updateinfo", "list", "--security"]).await {
            Some((0, sec)) => Some(count_dnf_advisory_packages(&sec)),
            _ => None,
        };
        return Some(split(total, security));
    }
    None
}

#[cfg(target_os = "macos")]
async fn os_check() -> Option<UpdateStatus> {
    // softwareupdate has no reliable security flag, so security stays unknown.
    let (_, out) = output("softwareupdate", &["--list"]).await?;
    Some(UpdateStatus {
        regular: Some(parse_softwareupdate(&out)),
        security: None,
    })
}

#[cfg(windows)]
async fn os_check() -> Option<UpdateStatus> {
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
    Some(split(total, Some(security)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn os_check() -> Option<UpdateStatus> {
    None
}

/// Regular = everything that isn't a security update.
fn split(total: u32, security: Option<u32>) -> UpdateStatus {
    UpdateStatus {
        regular: Some(total.saturating_sub(security.unwrap_or(0))),
        security,
    }
}

/// `apt-get -s upgrade`: one `Inst` line per package; the origin in brackets names the
/// suite, and `-security` suites carry security updates.
pub fn parse_apt(out: &str) -> UpdateStatus {
    let inst: Vec<&str> = out.lines().filter(|l| l.starts_with("Inst ")).collect();
    let security = inst.iter().filter(|l| l.contains("-security")).count() as u32;
    split(inst.len() as u32, Some(security))
}

/// `dnf -q check-update`: package lines are `name.arch  version  repo`; stop at the
/// "Obsoleting Packages" section, which repeats packages.
pub fn count_dnf_packages(out: &str) -> u32 {
    out.lines()
        .take_while(|l| !l.starts_with("Obsoleting"))
        .filter(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            cols.len() == 3 && cols[0].contains('.')
        })
        .count() as u32
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
}
