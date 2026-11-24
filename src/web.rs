use crate::config::Config;
use crate::db::{self, Db};
use crate::discovery::Discovery;
use crate::model::{self, Inventory, relative_age};
use axum::{
    Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
};
use maud::{Markup, PreEscaped, html};
use std::sync::Arc;
use time::macros::format_description;

/// Vendored HTMX, embedded in the binary so the page makes no external
/// asset or CDN requests.
const HTMX: &str = include_str!("../vendor/htmx.js");

/// Service state shared between handlers.
pub struct AppState {
    pub db: Arc<Db>,
    pub config: Config,
    pub discovery: Arc<dyn Discovery + Send + Sync>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(get_root))
        .route("/table", get(get_table))
}

/// Full Inventory page: frames, counts, table, honest empty state.
async fn get_root(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let inventory = inventory(&state.db).await;
    Html(render_page(&state.config, &inventory).into_string())
}

/// Table fragment for HTMX polling: just the `<table>`, nothing else.
async fn get_table(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let inventory = inventory(&state.db).await;
    Html(render_table(&inventory).into_string())
}

/// Read the Inventory off the database, off the async task pool. A failed
/// read never takes the page down: render what we know, which at this
/// milestone is nothing.
async fn inventory(db: &Arc<Db>) -> Inventory {
    let db = db.clone();
    match tokio::task::spawn_blocking(move || list_devices(&db)).await {
        Ok(Ok(inventory)) => inventory,
        _ => Inventory::empty(),
    }
}

fn list_devices(db: &Db) -> Result<Inventory, db::DbError> {
    let devices: Vec<model::Device> = db.list_devices()?;
    let now = model::now_epoch_seconds();
    let mut new_in_24h = 0usize;
    let devices = devices
        .into_iter()
        .inspect(|device| {
            if now.saturating_sub(device.first_seen) <= 86_400 {
                new_in_24h += 1;
            }
        })
        .collect::<Vec<model::Device>>();
    let total = devices.len();
    Ok(Inventory {
        devices,
        total,
        new_in_24h,
    })
}

fn render_page(config: &Config, inventory: &Inventory) -> Markup {
    let now = model::now_epoch_seconds();
    let empty_state = if inventory.devices.is_empty() {
        html! {
            p id="empty-state" {
                "No devices have been discovered yet — lanlord is waiting for its first Scan."
            }
        }
    } else {
        html! {}
    };
    html! {
        doctype;
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "lanlord — Inventory" }
                script { (PreEscaped(HTMX)) }
            }
            body {
                header {
                    h1 { "lanlord" }
                    p { "Inventory" }
                    p id="listen" { "Listening on " (&config.listen) }
                }
                main {
                    p id="total" { "Total devices: " (inventory.total) }
                    p id="new-in-24h" { "New in the last 24 hours: " (inventory.new_in_24h) }
                    (render_table(inventory))
                    (empty_state)
                    p id="refreshed-at" {
                        "Inventory last refreshed "
                        time datetime=(absolute_timestamp(now)) { (relative_age(0)) }
                    }
                }
                footer {
                    p { "Read-only. No auth. HTMX refreshes the table every 10 seconds." }
                }
            }
        }
    }
}

/// Just the `<table>`: swapped into the page by HTMX polling.
fn render_table(inventory: &Inventory) -> Markup {
    let now = model::now_epoch_seconds();
    html! {
        table {
            thead {
                tr {
                    th { "MAC" }
                    th { "IP" }
                    th { "Hostname" }
                    th { "First seen" }
                    th { "Last seen" }
                }
            }
            tbody {
                @for device in &inventory.devices {
                    tr {
                        td { (&device.mac) }
                        td { (&device.ip) }
                        td { (device.hostname.as_deref().unwrap_or("—")) }
                        td {
                            time data-epoch=(device.first_seen.to_string())
                                 datetime=(absolute_timestamp(device.first_seen)) {
                                (relative_age(now.saturating_sub(device.first_seen)))
                            }
                        }
                        td {
                            time data-epoch=(device.last_seen.to_string())
                                 datetime=(absolute_timestamp(device.last_seen)) {
                                (relative_age(now.saturating_sub(device.last_seen)))
                            }
                        }
                    }
                }
            }
        }
    }
}

/// ISO-8601 UTC rendering of `epoch_seconds`, a machine readable absolute
/// timestamp for `datetime` attributes.
fn absolute_timestamp(epoch_seconds: u64) -> String {
    let format = format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    match time::OffsetDateTime::from_unix_timestamp(epoch_seconds as i64) {
        Ok(datetime) => datetime.format(&format).expect("datetime formats"),
        Err(_) => epoch_seconds.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Config, DEFAULT_DATABASE_PATH, DEFAULT_LISTEN_ADDRESS, DEFAULT_SCAN_INTERVAL_SECONDS,
    };
    use crate::db::Db;
    use crate::discovery::fake::Scripted;
    use crate::discovery::{Discovery, Observation};
    use crate::scan::run_scan;
    use axum::body::to_bytes;
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn observation_with_hostname(mac: &str, ip: &str, hostname: &str) -> Observation {
        Observation {
            mac: mac.to_string(),
            ip: ip.parse().unwrap(),
            hostname: Some(hostname.to_string()),
        }
    }

    fn open_temp_db(name: &str) -> Arc<Db> {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&path);
        Arc::new(Db::open(&path).expect("test database opens"))
    }

    fn state_with(db: Arc<Db>) -> Arc<AppState> {
        Arc::new(AppState {
            db,
            config: Config {
                interface: "lo".to_string(),
                subnet: None,
                scan_interval: DEFAULT_SCAN_INTERVAL_SECONDS,
                listen: DEFAULT_LISTEN_ADDRESS.to_string(),
                database: DEFAULT_DATABASE_PATH.to_string(),
            },
            discovery: Arc::new(Scripted::new(Vec::new())),
        })
    }

    async fn render_table(state: Arc<AppState>) -> String {
        let app = router().with_state(state);
        let request = Request::builder()
            .uri("/table")
            .body(axum::body::Body::empty())
            .expect("request builds");
        let response = app.oneshot(request).await.expect("router responds");
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn table_renders_hostname_value_when_present() {
        let db = open_temp_db("lanlord.test.web.hostname-present.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            "aa:bb:cc:dd:ee:01",
            "10.0.0.1",
            Some("gateway.home.arpa"),
            now,
        )
        .expect("insert succeeds");
        let body = render_table(state_with(db)).await;
        assert!(
            body.contains("<th>Hostname</th>"),
            "table has a Hostname column: {body}"
        );
        assert!(
            body.contains("gateway.home.arpa"),
            "resolved hostname renders: {body}"
        );
    }

    #[tokio::test]
    async fn table_renders_empty_marker_when_hostname_unknown() {
        let db = open_temp_db("lanlord.test.web.hostname-marker.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device("aa:bb:cc:dd:ee:01", "10.0.0.1", None, now)
            .expect("insert succeeds");
        let body = render_table(state_with(db)).await;
        assert!(
            body.contains("<th>Hostname</th>"),
            "table has a Hostname column: {body}"
        );
        assert!(
            body.contains("—"),
            "empty marker renders for unknown hostname: {body}"
        );
        assert!(
            !body.contains("None"),
            "renders a marker, not a debug string: {body}"
        );
    }

    #[tokio::test]
    async fn hostname_updates_after_a_later_scan() {
        let db = open_temp_db("lanlord.test.web.hostname-update.sqlite");
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
        let before = render_table(state_with(db.clone())).await;
        assert!(
            before.contains("old.home.arpa"),
            "first hostname renders: {before}"
        );
        assert!(!before.contains("new.home.arpa"));

        run_scan(&db, &discovery, "lo")
            .await
            .expect("repeat scan succeeds");
        let after = render_table(state_with(db)).await;
        assert!(
            after.contains("new.home.arpa"),
            "updated hostname renders: {after}"
        );
        assert!(
            !after.contains("old.home.arpa"),
            "old hostname no longer renders: {after}"
        );
    }
}
