pub trait Resource {
    const NAME: &'static str;
}

pub mod cpu_info;
pub mod disk_info;
pub mod endpoint;
pub mod endpoint_detail;
pub mod mem_info;
pub mod net_info;
pub mod os_info;
