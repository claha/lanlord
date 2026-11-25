use axum::body::Body;
use axum::http::{Method, StatusCode};
use lanlord::db::Db;
use lanlord::discovery::{Discovery, Observation};
use lanlord::model;
use lanlord::web::{AppState, router};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower::ServiceExt;

struct EmptyDiscovery;

impl Discovery for EmptyDiscovery {
    fn scan<'a>(
        &'a self,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<Observation>, lanlord::discovery::DiscoveryError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Ok(vec![]) })
    }
}

fn empty_config() -> lanlord::config::Config {
    lanlord::config::Config {
        interface: "lo".to_string(),
        subnet: None,
        scan_interval: 300,
        listen: "127.0.0.1:0".to_string(),
        database: ":memory:".to_string(),
    }
}

fn app_with_devices(
    devices: &[(&str, &str, u64, u64)],
    last_scan_start: Option<u64>,
) -> axum::Router {
    let db = Db::open(":memory:").expect("in-memory database opens and runs migrations");
    for (mac, ip, _first_seen, last_seen) in devices {
        db.upsert_device(&model::Mac::new(*mac), ip, None, *last_seen)
            .expect("upsert succeeds");
    }
    if let Some(start) = last_scan_start {
        db.set_last_scan_start(start)
            .expect("set last scan start succeeds");
    }

    let state = AppState {
        db: Arc::new(db),
        config: empty_config(),
        discovery: Arc::new(EmptyDiscovery),
    };
    router().with_state(Arc::new(state))
}

async fn body_of(resp: http::Response<Body>) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("response body");
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

#[tokio::test]
async fn present_device_renders_as_present() {
    let now = model::now_epoch_seconds();
    // Device was seen in the latest scan: last_seen == last_scan_start -> present.
    let app = app_with_devices(&[("aa:bb:cc:dd:ee:01", "10.0.0.1", now, now)], Some(now));

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;
    assert!(html.contains("aa:bb:cc:dd:ee:01"), "device renders: {html}");
    assert!(
        html.contains("present"),
        "device seen in latest scan renders as present: {html}"
    );
    assert!(
        !html.contains("absent"),
        "device seen in latest scan does not render as absent: {html}"
    );
}

#[tokio::test]
async fn absent_device_renders_as_absent() {
    let now = model::now_epoch_seconds();
    // Device's last_seen is before the latest scan start -> absent.
    let app = app_with_devices(
        &[("aa:bb:cc:dd:ee:01", "10.0.0.1", now - 100, now - 100)],
        Some(now),
    );

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;
    assert!(
        html.contains("aa:bb:cc:dd:ee:01"),
        "absent device keeps its row: {html}"
    );
    assert!(
        html.contains("<td>absent</td>"),
        "device absent from latest scan renders as absent: {html}"
    );
    assert!(
        !html.contains("<td>present</td>"),
        "absent device does not render as present: {html}"
    );
}

#[tokio::test]
async fn presence_boundary_last_seen_equals_scan_start_is_present() {
    let now = model::now_epoch_seconds();
    // last_seen == last_scan_start is the boundary: present.
    let app = app_with_devices(&[("aa:bb:cc:dd:ee:01", "10.0.0.1", now, now)], Some(now));

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    let html = body_of(resp).await;
    assert!(
        html.contains("present"),
        "last_seen == scan start is present: {html}"
    );
}

#[tokio::test]
async fn presence_boundary_last_seen_one_second_before_scan_start_is_absent() {
    let now = model::now_epoch_seconds();
    // last_seen == last_scan_start - 1 is absent.
    let app = app_with_devices(
        &[("aa:bb:cc:dd:ee:01", "10.0.0.1", now - 1, now - 1)],
        Some(now),
    );

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    let html = body_of(resp).await;
    assert!(
        html.contains("absent"),
        "last_seen < scan start is absent: {html}"
    );
}

#[tokio::test]
async fn absent_device_keeps_its_row_and_old_last_seen() {
    let now = model::now_epoch_seconds();
    // Device A is present in the latest scan; device B was absent.
    let app = app_with_devices(
        &[
            ("aa:bb:cc:dd:ee:01", "10.0.0.1", now, now),
            ("aa:bb:cc:dd:ee:02", "10.0.0.2", now - 200, now - 200),
        ],
        Some(now),
    );

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    let html = body_of(resp).await;

    // Both devices render: absent device keeps its row.
    assert!(
        html.contains("aa:bb:cc:dd:ee:01"),
        "present device renders: {html}"
    );
    assert!(
        html.contains("aa:bb:cc:dd:ee:02"),
        "absent device keeps its row: {html}"
    );
    assert!(
        html.contains("present"),
        "present device renders as present: {html}"
    );
    assert!(
        html.contains("absent"),
        "absent device renders as absent: {html}"
    );
}

#[tokio::test]
async fn no_scan_has_run_all_devices_absent() {
    let now = model::now_epoch_seconds();
    // No last_scan_start recorded: no device can be present.
    let app = app_with_devices(&[("aa:bb:cc:dd:ee:01", "10.0.0.1", now, now)], None);

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    let html = body_of(resp).await;
    assert!(
        html.contains("absent"),
        "device is absent when no scan has run: {html}"
    );
}

#[tokio::test]
async fn new_in_24h_counts_devices_with_first_seen_within_24_hours() {
    let now = model::now_epoch_seconds();
    // Device A: first seen 1 hour ago (within 24h). Device B: first seen 25 hours ago (outside).
    let app = app_with_devices(
        &[
            ("aa:bb:cc:dd:ee:01", "10.0.0.1", now - 3_600, now - 3_600),
            ("aa:bb:cc:dd:ee:02", "10.0.0.2", now - 90_000, now - 90_000),
        ],
        Some(now),
    );

    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    let html = body_of(resp).await;

    assert!(
        html.contains("New in the last 24 hours: 1"),
        "exactly one device is new in 24h: {html}"
    );
    assert!(
        html.contains("Total devices: 2"),
        "total count is two: {html}"
    );
}
