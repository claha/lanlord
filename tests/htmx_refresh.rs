use axum::body::Body;
use axum::http::{Method, StatusCode};
use lanlord::config::Config;
use lanlord::db::Db;
use lanlord::discovery::{Discovery, Observation};
use lanlord::model::Mac;
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

fn empty_config() -> Config {
    Config {
        interface: "lo".to_string(),
        subnet: None,
        scan_interval: 300,
        listen: "127.0.0.1:0".to_string(),
        database: ":memory:".to_string(),
    }
}

async fn body_of(resp: http::Response<Body>) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("response body");
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

fn app_with_one_device() -> axum::Router {
    let db = Arc::new(Db::open(":memory:").expect("in-memory database opens and runs migrations"));
    db.upsert_device(
        &Mac::new("aa:bb:cc:dd:ee:ff"),
        "10.0.0.1",
        None,
        1_700_000_000,
    )
    .expect("seed device upserts");
    let state = AppState {
        db,
        config: empty_config(),
        discovery: Arc::new(EmptyDiscovery),
    };
    router().with_state(Arc::new(state))
}

/// GET /table is the HTMX fragment: just the table, and it carries the seeded
/// Device rows.
#[tokio::test]
async fn get_table_returns_fragment_with_device_rows() {
    let app = app_with_one_device();
    let req = http::Request::builder()
        .uri("/table")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;

    // No document wrapper: a fragment stands on its own.
    assert!(
        !html.contains("<!DOCTYPE"),
        "fragment omits doctype: {html}"
    );
    assert!(
        !html.contains("<html"),
        "fragment omits the html element: {html}"
    );
    assert!(
        !html.contains("<head"),
        "fragment omits the head element: {html}"
    );
    assert!(
        !html.contains("<body"),
        "fragment omits the body element: {html}"
    );

    // Device rows: the seeded device shows up in the register.
    assert!(html.contains("<table"), "fragment renders a table: {html}");
    assert!(
        html.contains("aa:bb:cc:dd:ee:ff"),
        "fragment shows the MAC: {html}"
    );
    assert!(html.contains("10.0.0.1"), "fragment shows the IP: {html}");
    for column in ["Last seen", "Device", "Address", "First seen", "State"] {
        assert!(
            html.contains(column),
            "fragment register has a {column} column: {html}"
        );
    }
}

/// GET / references only local assets: HTMX is served from the same host, no
/// CDN, no external fonts, and the table polls its own fragment.
#[tokio::test]
async fn get_root_references_only_local_assets_and_polls_the_fragment() {
    let app = app_with_one_device();
    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;

    // Vendored HTMX is embedded inline in the page: no external asset or CDN requests.
    assert!(
        html.contains("htmx"),
        "page embeds the vendored HTMX library inline: {html}"
    );
    assert!(
        !html.contains("src=\"http"),
        "no http(s) script src: {html}"
    );
    assert!(
        !html.contains("href=\"http"),
        "no http(s) link href: {html}"
    );
    assert!(!html.contains("unpkg"), "no unpkg CDN: {html}");
    assert!(!html.contains("cdnjs"), "no cdnjs CDN: {html}");

    // Phone width: the page opts into responsive layout.
    assert!(
        html.contains("width=device-width"),
        "viewport meta tag makes the page readable at phone width: {html}"
    );

    // The table swaps itself in place from /table every 10 seconds, carrying
    // the current filter so a poll never widens the slice the operator chose.
    assert!(
        html.contains("hx-get=\"/table?filter=all\""),
        "table polls the /table fragment with its filter: {html}"
    );
    assert!(
        html.contains("hx-trigger=\"every 10s\""),
        "table refreshes every 10 seconds: {html}"
    );
    assert!(
        html.contains("hx-swap=\"outerHTML\""),
        "table region swaps in place without a full reload: {html}"
    );
}
