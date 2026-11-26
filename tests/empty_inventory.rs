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
        html.contains("<b>0</b><span>known</span>"),
        "known-device figure is zero: {html}"
    );
    assert!(
        html.contains("<b>0</b><span>present</span>"),
        "present figure is zero: {html}"
    );
    assert!(
        html.contains("<b>0</b><span>arrivals · 24 h</span>"),
        "arrivals figure is zero: {html}"
    );
    assert!(
        html.contains("No devices yet"),
        "distinct empty state before the first scan: {html}"
    );
    assert!(
        html.contains("first device that replies is recorded here"),
        "empty state explains how the register fills: {html}"
    );
    assert!(
        !html.contains("<table"),
        "no empty table skeleton is drawn: {html}"
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
    assert!(
        html.contains("id=\"table-region\""),
        "fragment renders the swappable region: {html}"
    );
    assert!(
        html.contains("hx-get=\"/table?filter=all\""),
        "the region carries its own polling URL and filter: {html}"
    );
    assert!(
        html.contains("No devices yet"),
        "fragment carries the empty state: {html}"
    );
    assert!(
        html.contains("hx-swap-oob=\"outerHTML\""),
        "figures ride along out of band: {html}"
    );
}
