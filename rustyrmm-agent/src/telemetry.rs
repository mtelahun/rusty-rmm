//! Inventory and Samples: collected on timers and sent over the Session.

use std::time::{Duration, Instant, SystemTime};

use rustyrmm_proto::agent::v1::{agent_message, AgentMessage};
use rustyrmm_proto::common::v1::{
    CpuInfo, DiskInfo, DiskSample, InterfaceSample, Inventory, NetworkInterface, OsInfo, Sample,
};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Product, RefreshKind, System};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

/// Measures Samples. Disk and network counters are "since last refresh", so rates are those
/// counters over the time since the previous Sample.
pub struct Sampler {
    system: System,
    disks: Disks,
    networks: Networks,
    last: Instant,
}

impl Sampler {
    pub fn new() -> anyhow::Result<Self> {
        let mut system = System::new()?;
        system.refresh_cpu_usage();
        system.refresh_memory();
        Ok(Self {
            system,
            disks: Disks::new_with_refreshed_list()?,
            networks: Networks::new_with_refreshed_list()?,
            last: Instant::now(),
        })
    }

    pub fn sample(&mut self) -> Sample {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.disks.refresh(true);
        self.networks.refresh(true);
        let secs = self.last.elapsed().as_secs_f64().max(0.001);
        self.last = Instant::now();
        let rate = |bytes: u64| (bytes as f64 / secs) as u64;

        let mut disks: Vec<DiskSample> = self
            .disks
            .iter()
            .filter(|d| !d.is_read_only()) // e.g. snap squashfs images
            .map(|d| {
                let usage = d.usage();
                DiskSample {
                    // The mount point, not the device: one device can be mounted twice, and
                    // rollups group by this name.
                    name: d.mount_point().display().to_string(),
                    used_bytes: d.total_space().saturating_sub(d.available_space()),
                    read_bytes_per_sec: rate(usage.read_bytes),
                    write_bytes_per_sec: rate(usage.written_bytes),
                }
            })
            .collect();
        disks.sort_by(|a, b| a.name.cmp(&b.name));
        let mut interfaces: Vec<InterfaceSample> = (&self.networks)
            .into_iter()
            .map(|(name, n)| InterfaceSample {
                name: name.to_string_lossy().into_owned(),
                rx_bytes_per_sec: rate(n.received()),
                tx_bytes_per_sec: rate(n.transmitted()),
                link_speed_bps: n.transmit_link_speed(),
            })
            .collect();
        interfaces.sort_by(|a, b| a.name.cmp(&b.name));

        Sample {
            collected_at: Some(now()),
            cpu_usage_percent: self.system.global_cpu_usage(),
            memory_used_bytes: self.system.used_memory(),
            memory_total_bytes: self.system.total_memory(),
            disks,
            interfaces,
        }
    }
}

/// Collects the Inventory. Slow parts (update checks) run as child processes with a timeout.
pub async fn inventory() -> anyhow::Result<Inventory> {
    let mut inv = tokio::task::spawn_blocking(static_inventory).await??;
    let (virt_system, virt_role) = virtualization().await;
    if let Some(os) = inv.os.as_mut() {
        os.virt_system = virt_system;
        os.virt_role = virt_role;
    }
    inv.updates = Some(crate::updates::check().await);
    Ok(inv)
}

fn static_inventory() -> anyhow::Result<Inventory> {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    )?;
    let cpu = system.cpus().first();
    let (family, model) = cpu_family_model();
    let mut disks: Vec<DiskInfo> = Disks::new_with_refreshed_list()?
        .iter()
        .filter(|d| !d.is_read_only())
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            mount_point: d.mount_point().display().to_string(),
            total_bytes: d.total_space(),
        })
        .collect();
    disks.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    let networks = Networks::new_with_refreshed_list()?;
    let mut interfaces: Vec<NetworkInterface> = (&networks)
        .into_iter()
        .map(|(name, n)| {
            let mut ipv4: Vec<String> = Vec::new();
            let mut ipv6: Vec<String> = Vec::new();
            for ip in n.ip_networks() {
                let s = ip.addr.to_string();
                if ip.addr.is_ipv4() {
                    ipv4.push(s)
                } else {
                    ipv6.push(s)
                }
            }
            ipv4.sort();
            ipv6.sort();
            NetworkInterface {
                name: name.to_string_lossy().into_owned(),
                link_speed_bps: n.transmit_link_speed(),
                ipv4,
                ipv6,
            }
        })
        .collect();
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Inventory {
        collected_at: Some(now()),
        hostname: System::host_name().unwrap_or_default(),
        system_uuid: Product::uuid().unwrap_or_default(),
        timezone: iana_time_zone::get_timezone().unwrap_or_default(),
        os: Some(OsInfo {
            name: System::long_os_version()
                .or_else(|_| System::name())
                .unwrap_or_default(),
            family: std::env::consts::OS.into(),
            version: System::os_version().unwrap_or_default(),
            virt_system: None,
            virt_role: None,
        }),
        cpu: Some(CpuInfo {
            brand: cpu.map(|c| c.brand().trim().to_owned()).unwrap_or_default(),
            vendor: cpu.map(|c| c.vendor_id().to_owned()).unwrap_or_default(),
            family,
            model,
            core_count: System::physical_core_count().unwrap_or(0) as u32,
            thread_count: system.cpus().len() as u32,
        }),
        memory_total_bytes: system.total_memory(),
        disks,
        interfaces,
        updates: None,
        agent_version: env!("CARGO_PKG_VERSION").into(),
    })
}

/// Numeric CPU family and model, which only x86 CPUID provides.
fn cpu_family_model() -> (Option<u32>, Option<u32>) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if let Some(f) = raw_cpuid::CpuId::new().get_feature_info() {
        return (Some(f.family_id().into()), Some(f.model_id().into()));
    }
    (None, None)
}

/// (virtualization system, role). "none" means bare metal; absent means unknown.
async fn virtualization() -> (Option<String>, Option<String>) {
    #[cfg(target_os = "linux")]
    {
        // Prints the technology ("kvm", "docker", …) or "none"; exits non-zero for "none".
        let out = tokio::process::Command::new("systemd-detect-virt")
            .output()
            .await;
        if let Ok(out) = out {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !v.is_empty() {
                let role = (v != "none").then(|| "guest".to_owned());
                return (Some(v), role);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("sysctl")
            .args(["-n", "kern.hv_vmm_present"])
            .output()
            .await;
        if let Ok(out) = out {
            match String::from_utf8_lossy(&out.stdout).trim() {
                "1" => return (Some("vm".into()), Some("guest".into())),
                "0" => return (Some("none".into()), None),
                _ => {}
            }
        }
    }
    // shortcut: Windows virtualization detection (WMI HypervisorPresent) is not implemented;
    // it stays unknown there.
    (None, None)
}

/// Sends a Sample every `every`, until the Session closes.
pub async fn send_samples(tx: mpsc::Sender<AgentMessage>, every: Duration) {
    let Ok(mut sampler) = tokio::task::spawn_blocking(Sampler::new)
        .await
        .map_err(drop)
        .and_then(|r| r.map_err(|e| eprintln!("cannot measure Samples: {e:#}")))
    else {
        return;
    };
    let mut tick = interval(every);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tick.tick().await; // Rates need a full interval since the Sampler's first refresh.
    loop {
        tick.tick().await;
        let Ok((sample, back)) = tokio::task::spawn_blocking(move || {
            let s = sampler.sample();
            (s, sampler)
        })
        .await
        else {
            return;
        };
        sampler = back;
        if tx
            .send(message(agent_message::Message::Sample(sample)))
            .await
            .is_err()
        {
            return; // Session closed.
        }
    }
}

/// Sends the Inventory now, then every `every` if it changed (ignoring `collected_at`).
pub async fn send_inventory(tx: mpsc::Sender<AgentMessage>, every: Duration) {
    let mut last: Option<Inventory> = None;
    let mut tick = interval(every);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let inv = match inventory().await {
            Ok(inv) => inv,
            Err(e) => {
                eprintln!("cannot collect Inventory: {e:#}");
                continue;
            }
        };
        if last
            .as_ref()
            .is_some_and(|prev| same_apart_from_time(prev, &inv))
        {
            continue;
        }
        if tx
            .send(message(agent_message::Message::Inventory(inv.clone())))
            .await
            .is_err()
        {
            return; // Session closed.
        }
        last = Some(inv);
    }
}

fn same_apart_from_time(a: &Inventory, b: &Inventory) -> bool {
    Inventory {
        collected_at: None,
        ..a.clone()
    } == Inventory {
        collected_at: None,
        ..b.clone()
    }
}

fn now() -> prost_types::Timestamp {
    prost_types::Timestamp::from(SystemTime::now())
}

fn message(m: agent_message::Message) -> AgentMessage {
    AgentMessage { message: Some(m) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampler_measures_this_machine() {
        let mut sampler = Sampler::new().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let s = sampler.sample();
        assert!(s.memory_total_bytes > 0);
        assert!(s.memory_used_bytes <= s.memory_total_bytes);
        assert!((0.0..=100.0).contains(&s.cpu_usage_percent));
        assert!(s.collected_at.is_some());
    }

    // The static part only: the update check goes over the network and has its own tests.
    #[test]
    fn inventory_describes_this_machine() {
        let inv = static_inventory().unwrap();
        assert!(!inv.hostname.is_empty());
        let os = inv.os.unwrap();
        assert_eq!(os.family, std::env::consts::OS);
        let cpu = inv.cpu.unwrap();
        assert!(cpu.thread_count >= 1);
        assert!(inv.memory_total_bytes > 0);
    }

    #[test]
    fn unchanged_inventory_is_detected_despite_new_timestamp() {
        let a = Inventory {
            hostname: "h".into(),
            collected_at: Some(now()),
            ..Default::default()
        };
        let mut b = a.clone();
        b.collected_at = Some(prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        });
        assert!(same_apart_from_time(&a, &b));
        b.hostname = "other".into();
        assert!(!same_apart_from_time(&a, &b));
    }
}
