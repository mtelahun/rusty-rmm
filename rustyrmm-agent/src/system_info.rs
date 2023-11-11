use machine_uid::machine_id;
use rustyrmm_proto::endpoint_registration::{CpuInfo, Disk, DiskInfo, DiskType, MemInfo, OsInfo};
use rustyrmm_types::{data::OsInfoInternal, ids::MachineId};
use sysinfo::{CpuExt, DiskExt, NetworkExt, ProcessExt, System, SystemExt};
use time::{format_description, UtcOffset};

const STR_UNKNOWN: &str = "Unknown";

pub fn init_system() -> System {
    // Please note that we use "new_all" to ensure that all list of
    // components, network interfaces, disks and users are already
    // filled!
    System::new_all()
}

pub fn refresh_system(sys: &mut System) {
    sys.refresh_all();
}

pub fn get_hostname(sys: &System) -> Result<String, String> {
    // We display all disks' information:
    println!("=> disks:");
    for disk in sys.disks() {
        println!("{:?}", disk);
    }

    // Network interfaces name, data received and data transmitted:
    println!("=> networks:");
    for (interface_name, data) in sys.networks() {
        println!(
            "{}: {}/{} B",
            interface_name,
            data.received(),
            data.transmitted()
        );
    }

    // Display processes ID, name na disk usage:
    for (pid, process) in sys.processes() {
        println!("[{}] {} {:?}", pid, process.name(), process.disk_usage());
    }

    Ok(sys.host_name().unwrap_or(String::from("")))
}

pub fn get_system_id() -> Result<MachineId, String> {
    let id = match machine_id::get_machine_id() {
        Ok(id) => id,
        _ => String::from_utf8([0u8; 16].to_vec()).unwrap(),
    };
    Ok(MachineId::from(id))
}

pub fn get_os(sys: &System) -> Result<OsInfo, String> {
    let format = format_description::parse("UTC [offset_hour sign:mandatory]:[offset_minute]")
        .map_err(|_| "Timezone parse error")?;
    let local_offset = UtcOffset::current_local_offset()
        .unwrap()
        .format(&format)
        .unwrap_or(STR_UNKNOWN.to_string());
    let info = OsInfoInternal {
        name: sys.name().unwrap_or(STR_UNKNOWN.to_string()),
        os_ver: sys.os_version().unwrap_or(STR_UNKNOWN.to_string()),
        kernel_ver: sys.kernel_version().unwrap_or(STR_UNKNOWN.to_string()),
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

pub fn get_cpu(sys: &System) -> CpuInfo {
    let mut cpus = Vec::<rustyrmm_proto::endpoint_registration::Cpu>::new();
    for cpu in sys.cpus() {
        cpus.push(rustyrmm_proto::endpoint_registration::Cpu {
            name: cpu.name().to_string(),
            vendor_id: cpu.vendor_id().to_string(),
            brand: cpu.brand().to_string(),
            frequency: cpu.frequency().to_string(),
        })
    }

    CpuInfo {
        core_count: sys.physical_core_count().unwrap_or(0) as u32,
        thread_count: cpus.len() as u32,
        cpus,
    }
}

pub fn get_memory(sys: &System) -> MemInfo {
    MemInfo {
        total: sys.total_memory(),
        used: sys.used_memory(),
    }
}

pub fn get_disk(sys: &System) -> DiskInfo {
    let mut disks = Vec::<Disk>::new();
    for disk in sys.disks() {
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
