use std::convert::Infallible;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// A MAC address: a device's stable, link-layer identity (see ADR 0001).
///
/// A thin newtype around a formatted string like `"aa:bb:cc:dd:ee:ff"`, so the
/// inventory domain is not keyed on a plain `String`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mac(String);

impl Mac {
    /// Create a `Mac` from a formatted string like `"aa:bb:cc:dd:ee:ff"`.
    /// No validation is performed beyond storing the string.
    pub fn new(mac: impl Into<String>) -> Self {
        Self(mac.into())
    }
}

impl From<&str> for Mac {
    fn from(mac: &str) -> Self {
        Self(mac.to_string())
    }
}

impl From<String> for Mac {
    fn from(mac: String) -> Self {
        Self(mac)
    }
}

impl AsRef<str> for Mac {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Mac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Mac {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl PartialEq<str> for Mac {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Mac {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// A Device is identified by its MAC address; the IP holds the most recent
/// address observed for that MAC. `present` is derived at render time from the
/// most recent Scan start time: true when Last seen is at or after it.
#[derive(Debug)]
pub struct Device {
    pub mac: Mac,
    pub ip: String,
    pub hostname: Option<String>,
    pub first_seen: u64, // UTC epoch seconds
    pub last_seen: u64,  // UTC epoch seconds
    pub present: bool,   // true when seen in the most recent Scan
}

impl Device {
    /// Mark whether this Device answered the most recent Scan, given its start
    /// time. A Device is present when its Last seen is at or after the scan
    /// start; when no scan has run, every device is absent.
    pub fn derive_presence(&mut self, last_scan_start: Option<u64>) {
        self.present = last_scan_start.is_some_and(|start| self.last_seen >= start);
    }
}

/// The Inventory as presented on the web page.
pub struct Inventory {
    pub devices: Vec<Device>,
    pub total: usize,
    pub new_in_24h: usize,
}

impl Inventory {
    pub fn empty() -> Self {
        Self {
            devices: Vec::new(),
            total: 0,
            new_in_24h: 0,
        }
    }

    /// Derive Presence for every Device from the most recent Scan start time.
    pub fn derive_presence(&mut self, last_scan_start: Option<u64>) {
        for device in &mut self.devices {
            device.derive_presence(last_scan_start);
        }
    }

    /// Count Devices first seen within the last 24 hours (86_400 seconds).
    pub fn new_in_24h_count(&self, now: u64) -> usize {
        self.devices
            .iter()
            .filter(|device| now.saturating_sub(device.first_seen) <= 86_400)
            .count()
    }
}

/// Seconds since the Unix epoch (UTC) for "now".
pub fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is not before the Unix epoch")
        .as_secs()
}

/// Render `age_seconds` as a human readable relative age, e.g. "3 minutes ago".
pub fn relative_age(age_seconds: u64) -> String {
    let seconds = age_seconds;
    if seconds < 45 {
        return "just now".to_string();
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{minutes} minutes ago")
        };
    }

    let hours = minutes / 60;
    if hours < 24 {
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        };
    }

    let days = hours / 24;
    if days == 1 {
        "1 day ago".to_string()
    } else {
        format!("{days} days ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_age_renders_known_worked_examples() {
        assert_eq!(relative_age(5), "just now");
        assert_eq!(relative_age(44), "just now");
        assert_eq!(relative_age(60), "1 minute ago");
        assert_eq!(relative_age(90), "1 minute ago");
        assert_eq!(relative_age(120), "2 minutes ago");
        assert_eq!(relative_age(3600), "1 hour ago");
        assert_eq!(relative_age(7200), "2 hours ago");
        assert_eq!(relative_age(90000), "1 day ago");
        assert_eq!(relative_age(172800), "2 days ago");
    }

    #[test]
    fn mac_wraps_and_renders_a_formatted_address() {
        let mac = Mac::new("aa:bb:cc:dd:ee:ff");
        assert_eq!(mac.to_string(), "aa:bb:cc:dd:ee:ff");
        assert_eq!(mac.as_ref(), "aa:bb:cc:dd:ee:ff");
        assert_eq!(mac, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn mac_parses_from_str() {
        let mac: Mac = "11:22:33:44:55:66".parse().expect("mac parses");
        assert_eq!(mac, "11:22:33:44:55:66");
    }

    #[test]
    fn inventory_derive_presence_marks_devices_seen_in_latest_scan() {
        let now = 1_700_000_000;
        let mut inventory = Inventory {
            devices: vec![
                Device {
                    mac: Mac::new("aa:bb:cc:dd:ee:01"),
                    ip: "10.0.0.1".to_string(),
                    hostname: None,
                    first_seen: now,
                    last_seen: now,
                    present: false,
                },
                Device {
                    mac: Mac::new("aa:bb:cc:dd:ee:02"),
                    ip: "10.0.0.2".to_string(),
                    hostname: None,
                    first_seen: now - 100,
                    last_seen: now - 100,
                    present: false,
                },
            ],
            total: 2,
            new_in_24h: 0,
        };
        inventory.derive_presence(Some(now));
        assert!(inventory.devices[0].present);
        assert!(!inventory.devices[1].present);
    }

    #[test]
    fn inventory_new_in_24h_count() {
        let now = 1_700_000_000;
        let inventory = Inventory {
            devices: vec![
                Device {
                    mac: Mac::new("aa:bb:cc:dd:ee:01"),
                    ip: "10.0.0.1".to_string(),
                    hostname: None,
                    first_seen: now - 3_600,
                    last_seen: now - 3_600,
                    present: false,
                },
                Device {
                    mac: Mac::new("aa:bb:cc:dd:ee:02"),
                    ip: "10.0.0.2".to_string(),
                    hostname: None,
                    first_seen: now - 90_000,
                    last_seen: now - 90_000,
                    present: false,
                },
            ],
            total: 2,
            new_in_24h: 0,
        };
        assert_eq!(inventory.new_in_24h_count(now), 1);
    }

    #[test]
    fn inventory_derive_presence_without_a_scan_marks_everything_absent() {
        let mut inventory = Inventory {
            devices: vec![Device {
                mac: Mac::new("aa:bb:cc:dd:ee:01"),
                ip: "10.0.0.1".to_string(),
                hostname: None,
                first_seen: 1_700_000_000,
                last_seen: 1_700_000_000,
                present: true, // stale, should be overridden
            }],
            total: 1,
            new_in_24h: 0,
        };
        inventory.derive_presence(None);
        assert!(!inventory.devices[0].present);
    }
}
