use crate::db::Db;
use crate::discovery::{self, Discovery, Observation};
use crate::model::now_epoch_seconds;
use std::net::Ipv4Addr;
use std::sync::Arc;

/// Apply one Scan to the Inventory: collect observations from the `Discovery`
/// implementation, drop the scanner's own addresses, and upsert each remaining
/// observation by MAC. First seen is set only on first insert; Last seen and IP
/// are refreshed on every observation.
pub async fn run_scan(
    db: &Arc<Db>,
    discovery: &Arc<dyn Discovery + Send + Sync>,
    interface: &str,
) -> Result<Vec<Observation>, ScanError> {
    let now = now_epoch_seconds();
    let observations = discovery.scan().await.map_err(ScanError::Discovery)?;

    let own = match scanner_addresses(interface) {
        Ok(own) => own,
        Err(error) => {
            eprintln!(
                "lanlord: could not determine scanner's own addresses for {interface}: {error}; \
                 not excluding any addresses from the scan"
            );
            OwnAddresses::default()
        }
    };

    let observations = observations
        .into_iter()
        .filter(|obs| !own.matches(obs))
        .collect::<Vec<_>>();
    for obs in &observations {
        db.upsert_device(&obs.mac, &obs.ip.to_string(), obs.hostname.as_deref(), now)
            .map_err(ScanError::Inventory)?;
    }
    Ok(observations)
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("discovery scan failed: {0}")]
    Discovery(#[source] discovery::DiscoveryError),
    #[error("failed to update the inventory: {0}")]
    Inventory(#[source] crate::db::DbError),
}

/// The scanner's own interface addresses, which must never appear as Devices.
#[derive(Clone, Debug, Default)]
struct OwnAddresses {
    mac: String,
    ips: Vec<Ipv4Addr>,
}

impl OwnAddresses {
    fn matches(&self, observation: &Observation) -> bool {
        observation.mac == self.mac || self.ips.contains(&observation.ip)
    }
}

/// Read the MAC and IPv4 addresses assigned to `interface`.
fn scanner_addresses(interface: &str) -> Result<OwnAddresses, std::io::Error> {
    use std::ffi::CStr;

    let mut mac = String::new();
    let mut ips = Vec::new();

    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry = head;
        while !entry.is_null() {
            let item = &*entry;
            let name = CStr::from_ptr(item.ifa_name).to_string_lossy();
            if name == interface
                && !item.ifa_addr.is_null()
                && (*item.ifa_addr).sa_family as std::os::raw::c_int == libc::AF_INET
            {
                let socket = &*(item.ifa_addr as *const libc::sockaddr as *const libc::sockaddr_in);
                ips.push(Ipv4Addr::from(socket.sin_addr.s_addr.to_be_bytes()));
            }
            entry = item.ifa_next;
        }
        libc::freeifaddrs(head);

        // The interface's MAC address via ioctl(SIOCGIFHWADDR).
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd >= 0 {
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name_bytes = interface.as_bytes();
            if name_bytes.len() < ifr.ifr_name.len() {
                for (i, &byte) in name_bytes.iter().enumerate() {
                    ifr.ifr_name[i] = byte as i8;
                }
                if libc::ioctl(fd, libc::SIOCGIFHWADDR, &mut ifr) == 0 {
                    let hardware = ifr.ifr_ifru.ifru_hwaddr.sa_data;
                    mac = format!(
                        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        hardware[0] as u8,
                        hardware[1] as u8,
                        hardware[2] as u8,
                        hardware[3] as u8,
                        hardware[4] as u8,
                        hardware[5] as u8,
                    );
                }
            }
            let _ = libc::close(fd);
        }
    }

    Ok(OwnAddresses { mac, ips })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Config, DEFAULT_DATABASE_PATH, DEFAULT_LISTEN_ADDRESS, DEFAULT_SCAN_INTERVAL_SECONDS,
    };
    use crate::discovery::fake::Scripted;
    use crate::web::{self, AppState};
    use axum::body::to_bytes;
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn observation(mac: &str, ip: &str) -> Observation {
        Observation {
            mac: mac.to_string(),
            ip: ip.parse().unwrap(),
            hostname: None,
        }
    }

    fn observation_with_hostname(mac: &str, ip: &str, hostname: &str) -> Observation {
        Observation {
            mac: mac.to_string(),
            ip: ip.parse().unwrap(),
            hostname: Some(hostname.to_string()),
        }
    }

    fn test_config(interface: &str) -> Config {
        Config {
            interface: interface.to_string(),
            subnet: None,
            scan_interval: DEFAULT_SCAN_INTERVAL_SECONDS,
            listen: DEFAULT_LISTEN_ADDRESS.to_string(),
            database: DEFAULT_DATABASE_PATH.to_string(),
        }
    }

    fn open_temp_db(name: &str) -> Arc<Db> {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&path);
        Arc::new(Db::open(&path).expect("test database opens"))
    }

    #[tokio::test]
    async fn first_scan_inserts_devices_and_sets_first_and_last_seen() {
        let db = open_temp_db("lanlord.test.scan.first.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![vec![
            observation("aa:bb:cc:dd:ee:01", "10.0.0.1"),
            observation("aa:bb:cc:dd:ee:02", "10.0.0.2"),
        ]]));
        let before = now_epoch_seconds();
        let observed = run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        let after = now_epoch_seconds();
        assert_eq!(observed.len(), 2);
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 2);
        for device in &devices {
            assert!(
                device.first_seen >= before,
                "first seen not before the scan"
            );
            assert!(device.first_seen <= after, "first seen not after the scan");
            assert_eq!(
                device.first_seen, device.last_seen,
                "first and last seen agree on insert"
            );
            assert_eq!(
                device.hostname, None,
                "no hostname in the observation means none stored"
            );
        }
    }

    #[tokio::test]
    async fn first_scan_stores_hostname_from_observation() {
        let db = open_temp_db("lanlord.test.scan.hostname-first.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![vec![
            observation_with_hostname("aa:bb:cc:dd:ee:01", "10.0.0.1", "gateway.home.arpa"),
            observation("aa:bb:cc:dd:ee:02", "10.0.0.2"),
        ]]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 2);
        let gateway = devices
            .iter()
            .find(|d| d.mac == "aa:bb:cc:dd:ee:01")
            .unwrap();
        assert_eq!(
            gateway.hostname.as_deref(),
            Some("gateway.home.arpa"),
            "resolved hostname is stored"
        );
        let other = devices
            .iter()
            .find(|d| d.mac == "aa:bb:cc:dd:ee:02")
            .unwrap();
        assert_eq!(other.hostname, None, "missing hostname stays unknown");
    }

    #[tokio::test]
    async fn repeat_scan_updates_hostname_when_it_changes() {
        let db = open_temp_db("lanlord.test.scan.hostname-change.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "old.home.arpa",
            )],
            vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "new.home.arpa",
            )],
        ]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        run_scan(&db, &discovery, "lo")
            .await
            .expect("repeat scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("new.home.arpa"),
            "changed hostname is refreshed"
        );
    }

    #[tokio::test]
    async fn scan_without_hostname_stores_nothing_without_failing() {
        // A failed reverse lookup yields no hostname: the scan still succeeds
        // and the device row is written without disturbing its other fields.
        let db = open_temp_db("lanlord.test.scan.hostname-none.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "old.home.arpa",
            )],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.2")],
        ]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        run_scan(&db, &discovery, "lo")
            .await
            .expect("scan without hostname still succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].hostname, None, "failed lookup stores nothing");
        assert_eq!(devices[0].ip, "10.0.0.2", "most recent IP still wins");
    }

    #[tokio::test]
    async fn repeat_scan_updates_last_seen_without_duplicating_or_moving_first_seen() {
        let db = open_temp_db("lanlord.test.scan.repeat.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.2")],
        ]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        let first_seen = devices[0].first_seen;
        let last_seen_after_first = devices[0].last_seen;

        run_scan(&db, &discovery, "lo")
            .await
            .expect("repeat scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1, "no duplicate device is created");
        assert_eq!(devices[0].mac, "aa:bb:cc:dd:ee:01");
        assert_eq!(devices[0].ip, "10.0.0.2", "most recent IP wins");
        assert_eq!(
            devices[0].first_seen, first_seen,
            "first seen is never overwritten"
        );
        assert!(
            devices[0].last_seen >= last_seen_after_first,
            "last seen is refreshed"
        );
    }

    #[tokio::test]
    async fn device_absent_from_latest_scan_keeps_its_row() {
        let db = open_temp_db("lanlord.test.scan.absent.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![
                observation("aa:bb:cc:dd:ee:01", "10.0.0.1"),
                observation("aa:bb:cc:dd:ee:02", "10.0.0.2"),
            ],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
        ]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 2);
        let b_last_seen = devices
            .iter()
            .find(|d| d.mac == "aa:bb:cc:dd:ee:02")
            .unwrap()
            .last_seen;

        run_scan(&db, &discovery, "lo")
            .await
            .expect("second scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 2, "absent device keeps its row");
        let b = devices
            .iter()
            .find(|d| d.mac == "aa:bb:cc:dd:ee:02")
            .unwrap();
        assert_eq!(b.last_seen, b_last_seen, "absent device is not touched");
    }

    #[tokio::test]
    async fn device_returning_after_absence_updates_last_seen() {
        let db = open_temp_db("lanlord.test.scan.returning.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
            vec![],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
        ]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        let last_seen_before_absence = devices[0].last_seen;

        run_scan(&db, &discovery, "lo")
            .await
            .expect("absence scan succeeds");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        run_scan(&db, &discovery, "lo")
            .await
            .expect("return scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert!(
            devices[0].last_seen > last_seen_before_absence,
            "return refreshes last seen"
        );
    }

    #[tokio::test]
    async fn scan_excludes_the_scanners_own_addresses() {
        // "lo" carries 127.0.0.1 and the all-zeros MAC, both of which must be
        // dropped even when the discovery reports them.
        let db = open_temp_db("lanlord.test.scan.exclude-own.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![vec![
            observation("00:00:00:00:00:00", "127.0.0.1"),
            observation("aa:bb:cc:dd:ee:01", "10.0.0.1"),
        ]]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].mac, "aa:bb:cc:dd:ee:01");
        assert_eq!(devices[0].ip, "10.0.0.1");
    }

    #[tokio::test]
    async fn scan_failure_is_returned_and_the_existing_inventory_is_kept() {
        let db = open_temp_db("lanlord.test.scan.failure.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
            )]]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("first scan succeeds");

        // Exhaust the scripted outcomes so the next scan fails.
        let failing: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![]));
        let error = run_scan(&db, &failing, "lo").await.unwrap_err();
        assert!(
            error.to_string().contains("scan failed"),
            "error carries the discovery cause: {error}"
        );

        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1, "existing inventory is never discarded");
    }

    async fn render_body(state: Arc<AppState>, uri: &str) -> String {
        let app = web::router().with_state(state);
        let request = Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("request builds");
        let response = app.oneshot(request).await.expect("router responds");
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn root_page_shows_total_count_and_orders_by_last_seen_newest_first() {
        let db = open_temp_db("lanlord.test.scan.root-page.sqlite");
        // B is scanned after A, so B must sort first.
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
            )]]));
        run_scan(&db, &discovery, "lo").await.expect("scan A");
        // Space the scans a second apart so Last seen differs.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:02",
                "10.0.0.2",
            )]]));
        run_scan(&db, &discovery, "lo").await.expect("scan B");

        let state = Arc::new(AppState {
            db,
            config: test_config("lo"),
            discovery: Arc::new(Scripted::new(Vec::new())),
        });
        let body = render_body(state, "/").await;
        assert!(body.contains("Total devices: 2"));
        let b_position = body.find("aa:bb:cc:dd:ee:02").expect("B renders");
        let a_position = body.find("aa:bb:cc:dd:ee:01").expect("A renders");
        assert!(b_position < a_position, "newest last seen sorts first");
    }

    #[tokio::test]
    async fn table_fragment_renders_only_the_table() {
        let db = open_temp_db("lanlord.test.scan.table-fragment.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
            )]]));
        run_scan(&db, &discovery, "lo")
            .await
            .expect("scan succeeds");
        let state = Arc::new(AppState {
            db,
            config: test_config("lo"),
            discovery: Arc::new(Scripted::new(Vec::new())),
        });
        let body = render_body(state, "/table").await;
        assert!(body.contains("<table>"));
        assert!(!body.contains("<!doctype"), "fragment omits the document");
        assert!(!body.contains("<html>"), "fragment omits the document");
    }

    #[tokio::test]
    async fn root_page_shows_empty_state_before_any_scan() {
        let db = open_temp_db("lanlord.test.scan.empty-state.sqlite");
        let state = Arc::new(AppState {
            db,
            config: test_config("lo"),
            discovery: Arc::new(Scripted::new(Vec::new())),
        });
        let body = render_body(state, "/").await;
        assert!(body.contains("Total devices: 0"));
        assert!(
            body.contains("No devices have been discovered yet"),
            "renders the honest empty state"
        );
    }
}
