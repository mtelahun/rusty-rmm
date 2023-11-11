#[derive(Clone, Debug, Default)]
pub struct OsInfoInternal {
    pub name: String,
    pub os_ver: String,
    pub kernel_ver: String,
    pub virt_system: String,
    pub virt_role: String,
    pub tz: String,
}

impl OsInfoInternal {
    pub fn full_name(&self) -> String {
        format!("{} {}", self.name, self.os_ver)
    }

    pub fn os_family(&self) -> String {
        let family = match self.name.to_lowercase().as_str() {
            "ubuntu" => String::from("Debian"),
            "mac os/x" => String::from("Darwin"),
            "freebsd" => String::from("BSD"),
            "netbsd" => String::from("BSD"),
            "openbsd" => String::from("BSD"),
            "windows" => String::from("Windows"),
            _ => String::from("Unknown"),
        };

        family
    }
}
