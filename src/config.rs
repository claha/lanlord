use serde::Deserialize;
use std::ffi::CStr;
use std::fs;
use thiserror::Error;

/// Defaults documented for operators:
///   - listen: 127.0.0.1:8080
///   - database: /var/lib/lanlord/lanlord.db
///   - scan interval: 300 seconds
pub const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:8080";
pub const DEFAULT_DATABASE_PATH: &str = "/var/lib/lanlord/lanlord.db";
pub const DEFAULT_SCAN_INTERVAL_SECONDS: u64 = 300;

/// TOML configuration file. `interface` is required; everything else is
/// optional and gets a sensible default.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub interface: String,
    #[serde(default)]
    pub subnet: Option<String>,
    #[serde(default = "default_scan_interval")]
    pub scan_interval: u64,
    #[serde(default = "default_listen_address")]
    pub listen: String,
    #[serde(default = "default_database_path")]
    pub database: String,
}

#[derive(Debug, Clone)]
pub struct Cli {
    pub config: Option<String>,
    pub listen: Option<String>,
    pub database: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no --config given: lanlord requires a TOML config file")]
    MissingConfig,
    #[error("config file {0} is unreadable: {1}")]
    Unreadable(String, #[source] std::io::Error),
    #[error("config file {0} contains invalid TOML: {1}")]
    InvalidToml(String, #[source] toml::de::Error),
    #[error(
        "config file {0} is missing the required `interface` setting: name the interface to scan, e.g. interface = \"eth0\""
    )]
    MissingInterface(String),
    #[error("interface {0} does not exist on this host")]
    InterfaceMissing(String),
    #[error("interface {0} has no IPv4 address, so the scan segment cannot be derived")]
    NoIpv4(String),
    #[error("invalid command line argument: {0}")]
    Cli(String),
}

fn default_scan_interval() -> u64 {
    DEFAULT_SCAN_INTERVAL_SECONDS
}
fn default_listen_address() -> String {
    DEFAULT_LISTEN_ADDRESS.to_string()
}
fn default_database_path() -> String {
    DEFAULT_DATABASE_PATH.to_string()
}

/// Load the config file and apply CLI overrides (`--listen`, `--database`).
/// Validation of the interface itself is a separate step (`validate`) so the
/// error can be reported before anything is opened.
pub fn load(cli: &Cli) -> Result<Config, ConfigError> {
    let Some(path) = &cli.config else {
        return Err(ConfigError::MissingConfig);
    };
    let text =
        fs::read_to_string(path).map_err(|error| ConfigError::Unreadable(path.clone(), error))?;
    let mut config: Config =
        toml::from_str(&text).map_err(|error| ConfigError::InvalidToml(path.clone(), error))?;
    if config.interface.is_empty() {
        return Err(ConfigError::MissingInterface(path.clone()));
    }
    if let Some(listen) = &cli.listen {
        config.listen = listen.clone();
    }
    if let Some(database) = &cli.database {
        config.database = database.clone();
    }
    Ok(config)
}

/// Validate that the configured interface exists and carries an IPv4 address.
/// Failure here is fatal at startup.
pub fn validate(config: &Config) -> Result<(), ConfigError> {
    if !interface_exists(&config.interface) {
        return Err(ConfigError::InterfaceMissing(config.interface.clone()));
    }
    if !interface_has_ipv4(&config.interface) {
        return Err(ConfigError::NoIpv4(config.interface.clone()));
    }
    Ok(())
}

/// Parse `--config`, `--listen`, `--database` from the command line.
pub fn parse_cli() -> Result<Cli, ConfigError> {
    let mut config = None;
    let mut listen = None;
    let mut database = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = Some(take_value(&mut args, "--config")?),
            "--listen" => listen = Some(take_value(&mut args, "--listen")?),
            "--database" => database = Some(take_value(&mut args, "--database")?),
            other => return Err(ConfigError::Cli(format!("unknown argument {other:?}"))),
        }
    }
    Ok(Cli {
        config,
        listen,
        database,
    })
}

fn take_value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, ConfigError> {
    args.next()
        .ok_or_else(|| ConfigError::Cli(format!("{name} requires a value")))
}

/// Iterate the address family of every address assigned to `interface`.
fn interface_address_families(interface: &str) -> Vec<std::os::raw::c_int> {
    let mut families = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return families;
        }
        let mut entry = head;
        while !entry.is_null() {
            let item = &*entry;
            let name = CStr::from_ptr(item.ifa_name).to_string_lossy();
            if name == interface && !item.ifa_addr.is_null() {
                families.push((*item.ifa_addr).sa_family as std::os::raw::c_int);
            }
            entry = item.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    families
}

fn interface_exists(interface: &str) -> bool {
    !interface_address_families(interface).is_empty()
}

fn interface_has_ipv4(interface: &str) -> bool {
    interface_address_families(interface).contains(&libc::AF_INET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_config(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        fs::write(&path, content).expect("test config file is written");
        path
    }

    #[test]
    fn load_minimal_config_applies_defaults() {
        let path = write_config("lanlord.test.minimal.toml", "interface = \"lo\"\n");
        let cli = Cli {
            config: Some(path.to_string_lossy().into_owned()),
            listen: None,
            database: None,
        };
        let config = load(&cli).expect("minimal config loads");
        assert_eq!(config.interface, "lo");
        assert_eq!(config.subnet, None);
        assert_eq!(config.scan_interval, DEFAULT_SCAN_INTERVAL_SECONDS);
        assert_eq!(config.listen, DEFAULT_LISTEN_ADDRESS);
        assert_eq!(config.database, DEFAULT_DATABASE_PATH);
    }

    #[test]
    fn load_rejects_missing_interface() {
        let path = write_config("lanlord.test.no-interface.toml", "scan_interval = 60\n");
        let cli = Cli {
            config: Some(path.to_string_lossy().into_owned()),
            listen: None,
            database: None,
        };
        let error = load(&cli).unwrap_err();
        assert!(
            error.to_string().contains("interface"),
            "actionable message: {error}"
        );
    }

    #[test]
    fn load_accepts_subnet_override() {
        let path = write_config(
            "lanlord.test.subnet.toml",
            "interface = \"lo\"\nsubnet = \"10.0.0.0/8\"\n",
        );
        let cli = Cli {
            config: Some(path.to_string_lossy().into_owned()),
            listen: None,
            database: None,
        };
        let config = load(&cli).expect("config with subnet override loads");
        assert_eq!(config.subnet.as_deref(), Some("10.0.0.0/8"));
    }

    #[test]
    fn listen_and_database_cli_flags_override_config_file() {
        let path = write_config(
            "lanlord.test.overrides.toml",
            "interface = \"lo\"\nlisten = \"127.0.0.1:8080\"\ndatabase = \"/var/lib/lanlord/lanlord.db\"\n",
        );
        let cli = Cli {
            config: Some(path.to_string_lossy().into_owned()),
            listen: Some("9.9.9.9:9999".to_string()),
            database: Some("/tmp/lanlord-test.db".to_string()),
        };
        let config = load(&cli).expect("overrides apply");
        assert_eq!(config.listen, "9.9.9.9:9999");
        assert_eq!(config.database, "/tmp/lanlord-test.db");
    }

    #[test]
    fn validate_accepts_interface_with_ipv4() {
        let config = Config {
            interface: "lo".to_string(),
            subnet: None,
            scan_interval: 300,
            listen: DEFAULT_LISTEN_ADDRESS.to_string(),
            database: DEFAULT_DATABASE_PATH.to_string(),
        };
        validate(&config).expect("loopback has an IPv4 address");
    }

    #[test]
    fn validate_rejects_unknown_interface() {
        let config = Config {
            interface: "lanlord-does-not-exist-999".to_string(),
            subnet: None,
            scan_interval: 300,
            listen: DEFAULT_LISTEN_ADDRESS.to_string(),
            database: DEFAULT_DATABASE_PATH.to_string(),
        };
        let error = validate(&config).unwrap_err();
        assert!(
            error.to_string().contains("lanlord-does-not-exist-999"),
            "actionable message: {error}"
        );
    }
}
