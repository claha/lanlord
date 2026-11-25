use axum::body::Body;
use axum::http::{Method, StatusCode};
use lanlord::config::Config;
use lanlord::db::Db;
use lanlord::discovery::{Discovery, Observation};
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

fn app() -> axum::Router {
    let db = Db::open(":memory:").expect("in-memory database opens and runs migrations");
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
async fn get_root_renders_empty_inventory_page() {
    let app = app();
    let req = http::Request::builder()
        .uri("/")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;

    assert!(html.contains("lanlord"), "page is branded: {html}");
    assert!(
        html.contains("Total devices: 0"),
        "total count is zero: {html}"
    );
    assert!(
        html.contains("New in the last 24 hours: 0"),
        "new-in-24-hours count is zero: {html}"
    );
    assert!(
        html.contains("No devices have been discovered yet"),
        "distinct empty state before the first scan: {html}"
    );
    assert!(
        html.contains("waiting for its first Scan"),
        "empty state explains why it is empty: {html}"
    );
    for column in ["MAC", "IP", "First seen", "Last seen"] {
        assert!(html.contains(column), "table has a {column} column: {html}");
    }
    assert!(html.contains("<table"), "page renders a table");
    assert!(
        html.contains("<tbody></tbody>"),
        "table body is empty when no devices exist: {html}"
    );

    // HTMX is vendored and embedded: no external asset or CDN requests.
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
}

#[tokio::test]
async fn get_table_renders_table_fragment_only() {
    let app = app();
    let req = http::Request::builder()
        .uri("/table")
        .method(Method::GET)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("request succeeds");
    assert_eq!(resp.status(), StatusCode::OK);

    let html = body_of(resp).await;

    assert!(
        !html.contains("<!DOCTYPE"),
        "fragment omits the document type: {html}"
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
    assert!(html.contains("<table"), "fragment renders a table: {html}");
    assert!(
        html.contains("<tbody></tbody>"),
        "fragment table body is empty: {html}"
    );
    for column in ["MAC", "IP", "First seen", "Last seen"] {
        assert!(
            html.contains(column),
            "fragment table has a {column} column: {html}"
        );
    }
}
