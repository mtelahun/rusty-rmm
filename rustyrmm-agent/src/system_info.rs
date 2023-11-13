use machine_uid::machine_id;
use rustyrmm_proto::endpoint_registration::{
    CpuInfo, Disk, DiskInfo, DiskType, Ip4Addr, Ip6Addr, MemInfo, NetInfo, NetInterface, OsInfo,
    UpdateStatus,
};
use rustyrmm_types::{data::OsInfoInternal, ids::MachineId};
use sysinfo::{CpuExt, DiskExt, NetworkExt, ProcessExt, System, SystemExt};
use time::{format_description, UtcOffset};

const STR_UNKNOWN: &str = "Unknown";

#[derive(Debug)]
pub struct SystemInformation {
    sys: System,
}

impl SystemInformation {
    pub fn new() -> SystemInformation {
        // Please note that we use "new_all" to ensure that all list of
        // components, network interfaces, disks and users are already
        // filled!
        Self {
            sys: System::new_all(),
        }
    }

    pub fn _refresh(&mut self) {
        self.sys.refresh_all();
    }

    pub fn get_hostname(&self) -> Result<String, String> {
        // Network interfaces name, data received and data transmitted:
        println!("=> networks:");
        for (interface_name, data) in self.sys.networks() {
            println!(
                "{}: {}/{} B",
                interface_name,
                data.received(),
                data.transmitted()
            );
        }

        // Display processes ID, name na disk usage:
        for (pid, process) in self.sys.processes() {
            println!("[{}] {} {:?}", pid, process.name(), process.disk_usage());
        }

        Ok(self.sys.host_name().unwrap_or(String::from("")))
    }

    pub fn get_system_id(&self) -> Result<MachineId, String> {
        let id = match machine_id::get_machine_id() {
            Ok(id) => id,
            _ => String::from_utf8([0u8; 16].to_vec()).unwrap(),
        };
        Ok(MachineId::from(id))
    }

    pub fn get_os(&self) -> Result<OsInfo, String> {
        let format = format_description::parse("UTC [offset_hour sign:mandatory]:[offset_minute]")
            .map_err(|_| "Timezone parse error")?;
        let local_offset = UtcOffset::current_local_offset()
            .unwrap()
            .format(&format)
            .unwrap_or(STR_UNKNOWN.to_string());
        let info = OsInfoInternal {
            name: self.sys.name().unwrap_or(STR_UNKNOWN.to_string()),
            os_ver: self.sys.os_version().unwrap_or(STR_UNKNOWN.to_string()),
            kernel_ver: self.sys.kernel_version().unwrap_or(STR_UNKNOWN.to_string()),
            virt_system: STR_UNKNOWN.to_string(),
            virt_role: STR_UNKNOWN.to_string(),
            tz: local_offset,
        };

        Ok(OsInfo {
            full_name: info.full_name(),
            family: info.os_family(),
            version: info.os_ver,
            virt_system: info.virt_system,
            virt_role: info.virt_role,
            tz: info.tz,
        })
    }

    pub fn get_cpu(&self) -> CpuInfo {
        let mut cpus = Vec::<rustyrmm_proto::endpoint_registration::Cpu>::new();
        for cpu in self.sys.cpus() {
            cpus.push(rustyrmm_proto::endpoint_registration::Cpu {
                name: cpu.name().to_string(),
                vendor_id: cpu.vendor_id().to_string(),
                brand: cpu.brand().to_string(),
                frequency: cpu.frequency().to_string(),
            })
        }

        CpuInfo {
            core_count: self.sys.physical_core_count().unwrap_or(0) as u32,
            thread_count: cpus.len() as u32,
            cpus,
        }
    }

    pub fn get_memory(&self) -> MemInfo {
        MemInfo {
            total: self.sys.total_memory(),
            used: self.sys.used_memory(),
        }
    }

    pub fn get_disk(&self) -> DiskInfo {
        let mut disks = Vec::<Disk>::new();
        for disk in self.sys.disks() {
            disks.push(rustyrmm_proto::endpoint_registration::Disk {
                name: String::from(disk.name().to_str().unwrap_or("")),
                size: disk.total_space(),
                free: disk.available_space(),
                disk_type: match disk.kind() {
                    sysinfo::DiskKind::HDD => DiskType::TypeHdd as i32,
                    sysinfo::DiskKind::SSD => DiskType::TypeSsd as i32,
                    sysinfo::DiskKind::Unknown(_) => DiskType::TypeUnknown as i32,
                },
                filesystem: format!("{:?}", disk.file_system()),
                mount_point: String::from(disk.mount_point().to_str().unwrap_or("")),
            })
        }

        DiskInfo { disks }
    }

    pub fn get_network(&self) -> NetInfo {
        let mut ifs = Vec::<NetInterface>::new();
        for (if_name, if_data) in self.sys.networks() {
            ifs.push(NetInterface {
                name: if_name.to_string(),
                mac: if_data.mac_address().to_string(),
                ip4: Vec::<Ip4Addr>::new(),
                ip6: Vec::<Ip6Addr>::new(),
            })
        }

        NetInfo { ifs }
    }

    pub fn get_update(&self) -> UpdateStatus {
        UpdateStatus {
            security: 0,
            regular: 0,
        }
    }
}

impl Default for SystemInformation {
    fn default() -> Self {
        Self::new()
    }
}
