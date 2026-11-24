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
                    th { "First seen" }
                    th { "Last seen" }
                }
            }
            tbody {
                @for device in &inventory.devices {
                    tr {
                        td { (&device.mac) }
                        td { (&device.ip) }
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
