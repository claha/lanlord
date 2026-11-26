use crate::db::Db;
use crate::discovery::{self, Discovery, Observation};
use crate::interface::{format_mac, ipv4_addrs_of, mac_of};
use crate::model::now_epoch_seconds;
use std::net::Ipv4Addr;
use std::sync::Arc;

/// Bundles the state a Scan needs: the inventory database, the discovery
/// implementation, and the interface to scan. Passing it around keeps
/// `run_scan` and `run_scheduler` from taking the same arguments separately.
#[derive(Clone)]
pub struct ScanContext {
    pub db: Arc<Db>,
    pub discovery: Arc<dyn Discovery + Send + Sync>,
    pub interface: String,
}

/// Apply one Scan to the Inventory: collect observations from the `Discovery`
/// implementation, drop the scanner's own addresses, and upsert each remaining
/// observation by MAC. First seen is set only on first insert; Last seen and IP
/// are refreshed on every observation. `now` is the scan's start time and is
/// recorded as the last scan start time for presence derivation.
pub async fn run_scan(ctx: &ScanContext, now: u64) -> Result<Vec<Observation>, ScanError> {
    let observations = ctx.discovery.scan().await.map_err(ScanError::Discovery)?;

    let own = match scanner_addresses(&ctx.interface) {
        Ok(own) => own,
        Err(error) => {
            eprintln!(
                "lanlord: could not determine scanner's own addresses for {}: {error}; \
                 not excluding any addresses from the scan",
                ctx.interface
            );
            OwnAddresses::default()
        }
    };

    let observations = observations
        .into_iter()
        .filter(|obs| !own.matches(obs))
        .collect::<Vec<_>>();
    for obs in &observations {
        ctx.db
            .upsert_device(&obs.mac, &obs.ip.to_string(), obs.hostname.as_deref(), now)
            .map_err(ScanError::Inventory)?;
    }
    ctx.db
        .set_last_scan_start(now)
        .map_err(ScanError::Inventory)?;
    Ok(observations)
}

/// Run the Scan scheduler: one startup Scan immediately, then one every
/// `interval_secs` seconds. Scans never overlap — each Scan completes before
/// the next one is scheduled, so a long Scan delays the next instead of
/// stacking. Every Scan outcome (success or failure) is logged with its
/// duration; a failure is logged with its cause and does not stop the
/// scheduler, so the last known Inventory is kept. Runs forever; intended to
/// be spawned as a tokio task alongside the HTTP server.
pub async fn run_scheduler(ctx: ScanContext, interval_secs: u64) {
    // One startup Scan immediately.
    let now = now_epoch_seconds();
    run_and_log(&ctx, now).await;

    // Then one Scan every interval, never overlapping.
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;
        let now = now_epoch_seconds();
        run_and_log(&ctx, now).await;
    }
}

/// Run a single Scan and log its outcome and duration as structured `key=value`
/// lines to stdout (journald). A failure is logged with its cause and swallowed
/// so the scheduler keeps running and retries on the next interval; the
/// existing Inventory is never discarded.
async fn run_and_log(ctx: &ScanContext, now: u64) {
    let start = std::time::Instant::now();
    let result = run_scan(ctx, now).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(observations) => {
            let _ = ctx.db.set_last_scan_outcome("success");
            println!(
                "event=scan outcome=success duration_ms={} interface={} observations={}",
                duration_ms,
                ctx.interface,
                observations.len()
            );
        }
        Err(error) => {
            let _ = ctx.db.set_last_scan_outcome("failure");
            println!(
                "event=scan outcome=failure duration_ms={} interface={} cause={}",
                duration_ms, ctx.interface, error
            );
        }
    }
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
        observation.mac == self.mac.as_str() || self.ips.contains(&observation.ip)
    }
}

/// Read the scanner's own MAC and IPv4 addresses assigned to `interface`,
/// delegating to the shared helpers in `crate::interface`. A MAC that cannot
/// be read is degraded to an empty string (no exclusion) rather than failing
/// the whole scan; the caller logs that degradation.
fn scanner_addresses(interface: &str) -> Result<OwnAddresses, std::io::Error> {
    let mac = mac_of(interface).map(format_mac)?;
    let ips = ipv4_addrs_of(interface);
    Ok(OwnAddresses { mac, ips })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Config, DEFAULT_DATABASE_PATH, DEFAULT_LISTEN_ADDRESS, DEFAULT_SCAN_INTERVAL_SECONDS,
    };
    use crate::discovery::fake::Scripted;
    use crate::model::Mac;
    use crate::web::{self, AppState};
    use axum::body::to_bytes;
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    use crate::discovery::DiscoveryError;

    fn observation(mac: &str, ip: &str) -> Observation {
        Observation {
            mac: Mac::new(mac),
            ip: ip.parse().unwrap(),
            hostname: None,
        }
    }

    fn observation_with_hostname(mac: &str, ip: &str, hostname: &str) -> Observation {
        Observation {
            mac: Mac::new(mac),
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

    /// Build a `ScanContext` scanning the loopback interface over `db` and
    /// `discovery`, cloning the shared handles so the caller keeps using them.
    fn ctx_for(db: &Arc<Db>, discovery: &Arc<dyn Discovery + Send + Sync>) -> ScanContext {
        ScanContext {
            db: db.clone(),
            discovery: discovery.clone(),
            interface: "lo".to_string(),
        }
    }

    fn open_temp_db(name: &str) -> Arc<Db> {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&path);
        Arc::new(Db::open(&path).expect("test database opens"))
    }

    #[tokio::test]
    async fn run_scan_records_last_scan_start_in_meta() {
        let db = open_temp_db("lanlord.test.scan.meta.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
            )]]));
        let now = 1_700_000_000;
        run_scan(&ctx_for(&db, &discovery), now)
            .await
            .expect("scan succeeds");
        assert_eq!(
            db.last_scan_start().expect("read succeeds"),
            Some(now),
            "scan start time is recorded in meta"
        );
    }

    #[tokio::test]
    async fn first_scan_inserts_devices_and_sets_first_and_last_seen() {
        let db = open_temp_db("lanlord.test.scan.first.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![vec![
            observation("aa:bb:cc:dd:ee:01", "10.0.0.1"),
            observation("aa:bb:cc:dd:ee:02", "10.0.0.2"),
        ]]));
        let before = now_epoch_seconds();
        let observed = run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
    async fn scan_without_hostname_keeps_existing_hostname() {
        // A failed reverse lookup yields no hostname: the scan still succeeds
        // and the device row is written without disturbing its other fields.
        // A previously resolved hostname is preserved; only the IP is refreshed.
        let db = open_temp_db("lanlord.test.scan.hostname-none.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "old.home.arpa",
            )],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.2")],
        ]));
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("scan without hostname still succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("old.home.arpa"),
            "existing hostname is preserved when the lookup fails"
        );
        assert_eq!(devices[0].ip, "10.0.0.2", "most recent IP still wins");
    }

    #[tokio::test]
    async fn repeat_scan_updates_last_seen_without_duplicating_or_moving_first_seen() {
        let db = open_temp_db("lanlord.test.scan.repeat.sqlite");
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.2")],
        ]));
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        let first_seen = devices[0].first_seen;
        let last_seen_after_first = devices[0].last_seen;

        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 2);
        let b_last_seen = devices
            .iter()
            .find(|d| d.mac == "aa:bb:cc:dd:ee:02")
            .unwrap()
            .last_seen;

        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        let devices = db.list_devices().expect("list succeeds");
        let last_seen_before_absence = devices[0].last_seen;

        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("absence scan succeeds");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");

        // Exhaust the scripted outcomes so the next scan fails.
        let failing: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![]));
        let error = run_scan(&ctx_for(&db, &failing), now_epoch_seconds())
            .await
            .unwrap_err();
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("scan A");
        // Space the scans a second apart so Last seen differs.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:02",
                "10.0.0.2",
            )]]));
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
            .await
            .expect("scan B");

        let state = Arc::new(AppState {
            db,
            config: test_config("lo"),
            discovery: Arc::new(Scripted::new(Vec::new())),
        });
        let body = render_body(state, "/").await;
        assert!(
            body.contains("<b>2</b><span>known</span>"),
            "the known-device figure shows the total: {body}"
        );
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
        run_scan(&ctx_for(&db, &discovery), now_epoch_seconds())
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
        assert!(
            body.contains("<b>0</b><span>known</span>"),
            "the known-device figure reads zero: {body}"
        );
        assert!(
            body.contains("No devices yet"),
            "renders the honest empty state"
        );
    }

    /// A Discovery that blocks each `scan()` until `release()` is called, and
    /// counts how many times `scan()` was invoked.
    struct BlockingDiscovery {
        observations: Vec<Observation>,
        release: Arc<Notify>,
        call_count: AtomicUsize,
    }

    impl BlockingDiscovery {
        fn new(observations: Vec<Observation>) -> Self {
            Self {
                observations,
                release: Arc::new(Notify::new()),
                call_count: AtomicUsize::new(0),
            }
        }

        fn release(&self) {
            self.release.notify_waiters();
        }

        fn call_count(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl Discovery for BlockingDiscovery {
        fn scan<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Observation>, DiscoveryError>> + Send + 'a>>
        {
            let observations = self.observations.clone();
            let release = self.release.clone();
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                release.notified().await;
                Ok(observations)
            })
        }
    }

    #[tokio::test]
    async fn scheduler_runs_startup_scan_immediately() {
        tokio::time::pause();

        let db = open_temp_db("scheduler.startup.sqlite");
        let db_clone = db.clone();
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
            )]]));
        let discovery_clone = discovery.clone();

        let handle = tokio::task::spawn(run_scheduler(
            ScanContext {
                db: db_clone,
                discovery: discovery_clone,
                interface: "lo".to_string(),
            },
            300,
        ));

        // The startup scan runs without advancing time.
        tokio::task::yield_now().await;
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(
            devices.len(),
            1,
            "startup scan populated the inventory immediately"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn scheduler_runs_scan_every_interval() {
        tokio::time::pause();

        let db = open_temp_db("scheduler.interval.sqlite");
        let db_clone = db.clone();
        let discovery: Arc<dyn Discovery + Send + Sync> = Arc::new(Scripted::new(vec![
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.1")],
            vec![observation("aa:bb:cc:dd:ee:01", "10.0.0.2")],
        ]));
        let discovery_clone = discovery.clone();

        let handle = tokio::task::spawn(run_scheduler(
            ScanContext {
                db: db_clone,
                discovery: discovery_clone,
                interface: "lo".to_string(),
            },
            300,
        ));

        // Startup scan ran immediately.
        tokio::task::yield_now().await;
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "10.0.0.1");

        // Advance past the interval: the second scan runs and refreshes the IP.
        tokio::time::advance(Duration::from_secs(301)).await;
        tokio::task::yield_now().await;
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(
            devices[0].ip, "10.0.0.2",
            "second scan ran after the interval"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn scheduler_scans_never_overlap() {
        tokio::time::pause();

        let db = open_temp_db("scheduler.overlap.sqlite");
        let db_clone = db.clone();
        let discovery = Arc::new(BlockingDiscovery::new(vec![observation(
            "aa:bb:cc:dd:ee:01",
            "10.0.0.1",
        )]));
        let scheduler_discovery: Arc<dyn Discovery + Send + Sync> = discovery.clone();

        let handle = tokio::task::spawn(run_scheduler(
            ScanContext {
                db: db_clone,
                discovery: scheduler_discovery,
                interface: "lo".to_string(),
            },
            300,
        ));

        // Wait for the startup scan to start (it blocks on the discovery).
        while discovery.call_count() == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(discovery.call_count(), 1, "startup scan started");

        // Advance past the interval while the first scan is still blocked: no
        // second scan starts while the first is running.
        tokio::time::advance(Duration::from_secs(300)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            discovery.call_count(),
            1,
            "no second scan starts while the first is running"
        );

        // Release the first scan, let it complete and the scheduler loop back
        // to its sleep.
        discovery.release();
        while discovery.call_count() < 2 {
            tokio::time::advance(Duration::from_secs(300)).await;
            tokio::task::yield_now().await;
        }
        assert_eq!(
            discovery.call_count(),
            2,
            "second scan starts after the first completes"
        );

        handle.abort();
    }
}
