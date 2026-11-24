use std::time::{SystemTime, UNIX_EPOCH};

/// A Device is identified by its MAC address; the IP holds the most recent
/// address observed for that MAC.
#[derive(Debug)]
pub struct Device {
    pub mac: String,
    pub ip: String,
    pub hostname: Option<String>,
    pub first_seen: u64, // UTC epoch seconds
    pub last_seen: u64,  // UTC epoch seconds
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
}
