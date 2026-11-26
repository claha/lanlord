use crate::config::Config;
use crate::db::{self, Db};
use crate::discovery::Discovery;
use crate::model::{self, Inventory};
use axum::{
    Router,
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
};
use maud::{Markup, PreEscaped, html};
use serde::Deserialize;
use std::sync::Arc;
use time::macros::format_description;

/// Which slice of the register the operator asked for. A query parameter, not
/// client state, so filters survive the 10-second poll, the back button, and a
/// browser with JavaScript switched off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Present,
    Departed,
    Arrivals,
}

impl Filter {
    fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("present") => Self::Present,
            Some("departed") => Self::Departed,
            Some("arrivals") => Self::Arrivals,
            _ => Self::All,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Present => "present",
            Self::Departed => "departed",
            Self::Arrivals => "arrivals",
        }
    }

    /// Whether a Device belongs in this slice of the register.
    fn keeps(self, device: &model::Device, now: u64) -> bool {
        match self {
            Self::All => true,
            Self::Present => device.present,
            Self::Departed => !device.present,
            Self::Arrivals => is_arrival(device, now),
        }
    }

    /// The heading and one-line explanation of an empty slice, so an empty
    /// filter says what is true instead of showing a blank table.
    fn empty_copy(self) -> (&'static str, &'static str) {
        match self {
            Self::All => (
                "No devices yet",
                "Nothing has answered on this segment yet. lanlord keeps scanning; the first \
                 device that replies is recorded here and stays in the register.",
            ),
            Self::Present => (
                "Nothing present right now",
                "Everything in the register answered an earlier scan and has since gone quiet. \
                 The departed devices are still here, under Departed or All.",
            ),
            Self::Departed => (
                "Nothing has departed",
                "Every device in the register is on the segment right now. Nothing has gone quiet \
                 since the last scan.",
            ),
            Self::Arrivals => (
                "No arrivals in 24 hours",
                "No device appeared for the first time in the last day. The full register is \
                 under All.",
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TableQuery {
    filter: Option<String>,
}

/// A device seen for the first time within the last day: the change the
/// operator most wants to know about.
const ARRIVAL_WINDOW_SECONDS: u64 = 24 * 60 * 60;

fn is_arrival(device: &model::Device, now: u64) -> bool {
    now.saturating_sub(device.first_seen) < ARRIVAL_WINDOW_SECONDS
}

/// Vendored HTMX, embedded in the binary so the page makes no external
/// asset or CDN requests.
const HTMX: &str = include_str!("../vendor/htmx.js");

/// How often the full page polls the table fragment. A plain constant for
/// this ticket; not yet exposed as configuration.
const TABLE_POLL_INTERVAL_SECONDS: u64 = 10;

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

/// Full Inventory page: frames, counts, table region, honest empty state.
async fn get_root(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TableQuery>,
) -> impl IntoResponse {
    let now = model::now_epoch_seconds();
    let (inventory, health) = load_page(&state.db, now, state.config.scan_interval).await;
    let filter = Filter::parse(query.filter.as_deref());
    Html(render_page(&state.config, &inventory, &health, filter).into_string())
}

/// Table fragment for HTMX polling: the register plus its empty state, with
/// the figures row riding along as an out-of-band swap. Keeping the figures out
/// of band leaves the filter links outside the swap, so keyboard focus is not
/// stolen every ten seconds.
async fn get_table(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TableQuery>,
) -> impl IntoResponse {
    let now = model::now_epoch_seconds();
    let (inventory, health) = load_page(&state.db, now, state.config.scan_interval).await;
    let filter = Filter::parse(query.filter.as_deref());
    Html(render_table_fragment(&inventory, &health, filter).into_string())
}

/// The Inventory plus the scan start time the page needs to say how fresh it
/// is. Scan metadata belongs to the page, not to the domain Inventory.
async fn load_page(db: &Arc<Db>, now: u64, scan_interval: u64) -> (Inventory, ScanHealth) {
    let db_for_start = db.clone();
    let last_scan_start =
        match tokio::task::spawn_blocking(move || db_for_start.last_scan_start()).await {
            Ok(Ok(start)) => start,
            _ => None,
        };
    let db_for_outcome = db.clone();
    let last_scan_outcome =
        match tokio::task::spawn_blocking(move || db_for_outcome.last_scan_outcome()).await {
            Ok(Ok(outcome)) => outcome,
            _ => None,
        };
    let db_for_list = db.clone();
    let inventory =
        match tokio::task::spawn_blocking(move || list_devices(&db_for_list, last_scan_start, now))
            .await
        {
            Ok(Ok(inventory)) => inventory,
            _ => Inventory::empty(),
        };
    (
        inventory,
        ScanHealth {
            last_scan_start,
            last_scan_outcome,
            scan_interval,
            now,
        },
    )
}

/// Everything the page needs to say how fresh the scan data is.
#[derive(Clone, Debug)]
struct ScanHealth {
    last_scan_start: Option<u64>,
    last_scan_outcome: Option<String>,
    scan_interval: u64,
    now: u64,
}

impl ScanHealth {
    /// True when no scan has run, the last scan failed, or the last scan is
    /// older than roughly twice the scan interval — meaning the data on the
    /// board may not reflect the current segment.
    fn is_unhealthy(&self) -> bool {
        match self.last_scan_start {
            None => true,
            Some(start) => {
                let age = self.now.saturating_sub(start);
                self.last_scan_outcome.as_deref() == Some("failure") || age > self.scan_interval * 2
            }
        }
    }

    /// Seconds until the next scan is due, or None if no scan has run yet.
    fn next_scan_in(&self) -> Option<u64> {
        match self.last_scan_start {
            None => None,
            Some(start) => {
                let elapsed = self.now.saturating_sub(start);
                if elapsed >= self.scan_interval {
                    Some(0)
                } else {
                    Some(self.scan_interval - elapsed)
                }
            }
        }
    }
}

/// What the masthead calls the scanned range: the configured subnet when the
/// operator gave one, otherwise just the interface it scans.
fn segment_label(config: &Config) -> String {
    match &config.subnet {
        Some(subnet) => format!("segment {subnet} · {}", config.interface),
        None => format!("interface {} · subnet from the address", config.interface),
    }
}

fn list_devices(db: &Db, last_scan_start: Option<u64>, now: u64) -> Result<Inventory, db::DbError> {
    let devices = db.list_devices()?;
    let total = devices.len();
    let mut inventory = Inventory {
        devices,
        total,
        new_in_24h: 0,
    };
    // Presence and the new-in-24h count belong on the domain type, not in the
    // web layer: derive them on the Inventory and keep web.rs thin.
    inventory.derive_presence(last_scan_start);
    inventory.new_in_24h = inventory.new_in_24h_count(now);
    Ok(inventory)
}

fn render_page(
    config: &Config,
    inventory: &Inventory,
    health: &ScanHealth,
    filter: Filter,
) -> Markup {
    let segment = segment_label(config);
    html! {
        doctype;
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="dark light";
                title { "lanlord — Segment register" }
                // Vendored HTMX, embedded inline so the page makes no external
                // asset or CDN requests.
                script { (PreEscaped(&HTMX)) }
                // Keyboard accelerators for the filter slices: 1=All, 2=Present,
                // 3=Departed, 4=Arrivals · 24 h.
                script {
                    (PreEscaped(r#"
document.addEventListener('keydown', (event) => {
  if (event.ctrlKey || event.altKey || event.metaKey) return;
  const key = event.key;
  if (key >= '1' && key <= '4') {
    event.preventDefault();
    const filters = ['all', 'present', 'departed', 'arrivals'];
    window.location.href = '/?filter=' + filters[Number(key) - 1];
  }
});
"#))
                }
                style { (page_css()) }
            }
            body {
                div .board {
                    header .masthead {
                        div .brand {
                            h1 .wordmark { "lanlord" }
                            p .segment { (segment) }
                        }
                        p .clock {
                            span .clock-value { (TABLE_POLL_INTERVAL_SECONDS) " s" }
                            span .clock-label { "between refreshes" }
                        }
                    }
                    main {
                        (render_figures(inventory, false))
                        nav .filters aria-label="Register slices" {
                            (filter_link(Filter::All, "All", filter))
                            (filter_link(Filter::Present, "Present", filter))
                            (filter_link(Filter::Departed, "Departed", filter))
                            (filter_link(Filter::Arrivals, "Arrivals · 24 h", filter))
                        }
                        (render_polling_table(inventory, health, filter))
                    }
                    footer .colophon {
                        p {
                            "Read-only. No authentication. "
                            (inventory.total) " devices recorded; history is kept for everything \
                             that ever answered."
                        }
                    }
                }
            }
        }
    }
}

/// The figures row: the four numbers the operator checks first. It renders
/// inline on load and out of band on every poll, so counts never lag the table
/// they describe, and it never sits inside the swappable region (which would
/// cost keyboard focus on the filter links every ten seconds).
fn render_figures(inventory: &Inventory, out_of_band: bool) -> Markup {
    let present = inventory
        .devices
        .iter()
        .filter(|device| device.present)
        .count();
    let departed = inventory.total - present;
    html! {
        section id="figures" class="figures"
            hx-swap-oob=[if out_of_band { Some("outerHTML") } else { None }] {
            div .figure {
                b { (inventory.total) }
                span { "known" }
            }
            div .figure {
                b { (present) }
                span { "present" }
            }
            div .figure {
                b { (departed) }
                span { "departed" }
            }
            div .figure .figure-arrival {
                b { (inventory.new_in_24h) }
                span { "arrivals · 24 h" }
            }
        }
    }
}

/// A filter is a real link, so the slice survives the poll, the back button,
/// and a browser with JavaScript switched off.
fn filter_link(target: Filter, label: &str, current: Filter) -> Markup {
    let is_current = target == current;
    html! {
        a class=(if is_current { "key is-current" } else { "key" })
            href=(format!("/?filter={}", target.as_str()))
            aria-current=[if is_current { Some("page") } else { None }] {
            (label)
        }
    }
}

/// One device row: last-seen time leads, the device carries its MAC, and a
/// state flap closes the row.
fn render_row(device: &model::Device, now: u64) -> Markup {
    let arrival = is_arrival(device, now);
    let hostname = device.hostname.as_deref();
    html! {
        tr class=(if device.present { "row present" } else { "row departed" }) {
            td .col-time {
                time datetime=(absolute_timestamp(device.last_seen))
                     title=(absolute_timestamp(device.last_seen)) {
                    (board_age(device.last_seen, now, device.present))
                }
            }
            td .col-device {
                span class=(if hostname.is_some() { "device-name" } else { "device-name no-name" }) {
                    (hostname.unwrap_or("no hostname"))
                }
                @if arrival {
                    span .flag { "arrival" }
                }
                span .device-mac { (&device.mac) }
                span .device-addr { (device.ip) }
            }
            td .col-addr { (&device.ip) }
            td .col-first {
                time datetime=(absolute_timestamp(device.first_seen))
                     title=(absolute_timestamp(device.first_seen)) {
                    (board_age(device.first_seen, now, false))
                }
            }
            td .col-state {
                span class=(if device.present { "flap" } else { "flap flap-departed" }) {
                    @if device.present { "Present" } @else { "Departed" }
                }
            }
        }
    }
}

/// The register itself, with the departed group cut off by one structural rule
/// when the full slice is on screen: absence is a place on the page, not only a
/// filter.
fn render_register(inventory: &Inventory, health: &ScanHealth, filter: Filter) -> Markup {
    let rows: Vec<&model::Device> = inventory
        .devices
        .iter()
        .filter(|device| filter.keeps(device, health.now))
        .collect();
    if rows.is_empty() {
        let has_scanned = health.last_scan_start.is_some();
        let (title, body) = if !has_scanned {
            match filter {
                Filter::All => (
                    "No devices yet",
                    "lanlord hasn't finished its first scan yet. It scans on startup \
                     and every {} seconds; the first device that replies is recorded \
                     here and stays in the register.",
                ),
                Filter::Present => (
                    "Nothing present right now",
                    "No scan has completed yet, so nothing is known to be on the \
                     segment. The first scan runs on startup and every {} seconds.",
                ),
                Filter::Departed => (
                    "Nothing has departed",
                    "No scan has completed yet, so nothing is known to have gone \
                     quiet. The first scan runs on startup and every {} seconds.",
                ),
                Filter::Arrivals => (
                    "No arrivals in 24 hours",
                    "No scan has completed yet, so no first-time appearances are \
                     recorded. The first scan runs on startup and every {} seconds.",
                ),
            }
        } else {
            filter.empty_copy()
        };
        let body = if !has_scanned {
            body.replace("{}", &health.scan_interval.to_string())
        } else {
            body.to_string()
        };
        return html! {
            div .empty {
                h2 { (title) }
                p { (body) }
                @if filter != Filter::All {
                    p .empty-action { a href="/?filter=all" { "Show the full register" } }
                }
            }
        };
    }
    let departed_total = inventory
        .devices
        .iter()
        .filter(|device| !device.present)
        .count();
    html! {
        div .register {
            table {
                caption .visually-hidden {
                    "Devices on the segment, most recently seen first"
                }
                thead {
                    tr {
                        th .col-time { "Last seen" }
                        th .col-device { "Device" }
                        th .col-addr { "Address" }
                        th .col-first { "First seen" }
                        th .col-state { "State" }
                    }
                }
                tbody {
                    @for (index, device) in rows.iter().enumerate() {
                        @if filter == Filter::All
                            && !device.present
                            && index > 0
                            && rows[index - 1].present {
                            tr .divider {
                                td colspan="5" { "Departed · " (departed_total) }
                            }
                        }
                        (render_row(device, health.now))
                    }
                }
            }
        }
    }
}

/// How fresh the data is, said plainly, with a sweep that restarts on each
/// poll so the page shows it is alive without a spinner. When the scan is
/// stale or failed, an amber health warning appears so the operator can tell
/// a quiet segment from a broken scan.
fn render_status_strip(inventory: &Inventory, health: &ScanHealth, filter: Filter) -> Markup {
    let shown = inventory
        .devices
        .iter()
        .filter(|device| filter.keeps(device, health.now))
        .count();
    html! {
        p .strip {
            span .strip-scan {
                @match health.last_scan_start {
                    Some(start) => { "last scan " (scan_age(health.now.saturating_sub(start))) },
                    None => { "no scan has completed yet" },
                }
            }
            @if health.is_unhealthy() {
                span .strip-health {
                    @match health.last_scan_outcome.as_deref() {
                        Some("failure") => { "scan failed" }
                        _ => { "stale data" }
                    }
                }
            }
            @if let Some(seconds) = health.next_scan_in() {
                span .strip-eta {
                    "next scan in " (scan_age(seconds))
                }
            }
            span .strip-count {
                (shown) " of " (inventory.total) " shown"
            }
            span .sweep aria-hidden="true" {}
        }
    }
}

/// Table fragment for HTMX polling: figures (out of band) plus the region that
/// replaces itself, so table and empty state always change together.
fn render_table_fragment(inventory: &Inventory, health: &ScanHealth, filter: Filter) -> Markup {
    html! {
        (render_figures(inventory, true))
        (render_polling_table(inventory, health, filter))
    }
}

/// The self-replacing region: HTMX polls `/table` (carrying the current filter)
/// and swaps this div in place, without ever reloading the document.
fn render_polling_table(inventory: &Inventory, health: &ScanHealth, filter: Filter) -> Markup {
    html! {
        div id="table-region" .region
            hx-get=(format!("/table?filter={}", filter.as_str()))
            hx-trigger=(format!("every {TABLE_POLL_INTERVAL_SECONDS}s"))
            hx-swap="outerHTML"
        {
            (render_status_strip(inventory, health, filter))
            (render_register(inventory, health, filter))
        }
    }
}

/// The board's stylesheet: system faces only, no external requests. Dark is the
/// board's native appearance; light is a daylight board of equal craft, chosen
/// by the OS, never a reduced fallback.
fn page_css() -> Markup {
    html! {
        ":root{color-scheme:dark light;\
        --sans:system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;\
        --mono:ui-monospace,'SF Mono',Menlo,'Cascadia Mono','DejaVu Sans Mono',monospace;\
        --ground:#15161a;--text:#e9e5da;--text-dim:#9c968a;--rule:#33353b;--rule-strong:#4d5057;\
        --amber:#eda23a;--flap-bg:#e9e5da;--flap-ink:#15161a;--flap-dep-ink:#9c968a;\
        --seam:rgba(0,0,0,.28);--row-hover:rgba(255,255,255,.035);--edge:inset 0 0 0 1px #2a2c31}\
        @media (prefers-color-scheme:light){:root{--ground:#eceef0;--text:#191b1e;--text-dim:#5f656c;\
        --rule:#d2d6da;--rule-strong:#adb3b9;--amber:#875000;--flap-bg:#191b1e;--flap-ink:#f2f4f6;\
        --flap-dep-ink:#5f656c;--seam:rgba(255,255,255,.26);--row-hover:rgba(0,0,0,.035);\
        --edge:inset 0 0 0 1px #dde1e4}}\
        *{box-sizing:border-box}\
        body{margin:0;background:var(--ground);color:var(--text);font-family:var(--sans);\
        font-size:15px;line-height:1.45;-webkit-font-smoothing:antialiased;scrollbar-color:var(--rule-strong) var(--ground)}\
        ::selection{background:var(--amber);color:var(--ground)}\
        a{color:inherit}\
        a:focus-visible{outline:2px solid var(--amber);outline-offset:2px}\
        .board{max-width:1040px;margin:0 auto;padding:26px 28px 40px;box-shadow:var(--edge);min-height:100vh}\
        .masthead{display:flex;align-items:flex-end;justify-content:space-between;gap:24px;\
        padding-bottom:14px;border-bottom:1px solid var(--rule-strong)}\
        .wordmark{margin:0;font-size:20px;font-weight:700;letter-spacing:.34em;text-transform:uppercase}\
        .segment{margin:8px 0 0;font-family:var(--mono);font-size:12px;letter-spacing:.03em;color:var(--text-dim)}\
        .clock{margin:0;text-align:right;font-family:var(--mono);color:var(--amber)}\
        .clock-value{display:block;font-size:26px;line-height:1}\
        .clock-label{display:block;margin-top:4px;font-size:11px;letter-spacing:.12em;\
        text-transform:uppercase;color:var(--text-dim)}\
        .figures{display:grid;grid-template-columns:repeat(4,1fr);margin:0;border-bottom:1px solid var(--rule)}\
        .figure{padding:20px 0 16px;border-right:1px solid var(--rule)}\
        .figure:last-child{border-right:0}\
        .figure b{display:block;font-size:40px;font-weight:700;line-height:.9;letter-spacing:-.02em;\
        font-variant-numeric:tabular-nums}\
        .figure span{display:block;margin-top:10px;font-family:var(--mono);font-size:11px;\
        letter-spacing:.12em;text-transform:uppercase;color:var(--text-dim)}\
        .figure-arrival b,.figure-arrival span{color:var(--amber)}\
        .filters{display:flex;flex-wrap:wrap;margin:18px 0 0;border:1px solid var(--rule-strong);\
        width:max-content;max-width:100%}\
        .key{padding:7px 16px;border-right:1px solid var(--rule);color:var(--text);\
        text-decoration:none;font-family:var(--mono);font-size:11px;letter-spacing:.12em;text-transform:uppercase}\
        .key:last-child{border-right:0}\
        .key:hover{background:var(--row-hover)}\
        .key.is-current{background:var(--flap-bg);color:var(--flap-ink)}\
        .key:focus-visible{outline:2px solid var(--amber);outline-offset:-2px}\
        .strip{position:relative;display:flex;justify-content:space-between;gap:16px;margin:0;\
        padding:9px 0 11px;border-bottom:1px solid var(--rule);font-family:var(--mono);font-size:11px;\
        letter-spacing:.1em;text-transform:uppercase;color:var(--text-dim);overflow:hidden}\
        .strip-health{color:var(--amber);font-weight:600}\
        .strip-eta{color:var(--text-dim)}\
        .sweep{position:absolute;left:0;bottom:0;width:100%;height:1px;background:var(--text-dim);opacity:.5;\
        transform-origin:left;animation:sweep 10s linear infinite}\
        @keyframes sweep{from{transform:scaleX(1)}to{transform:scaleX(0)}}\
        @media (prefers-reduced-motion:reduce){.sweep{animation:none;opacity:.35}}\
        .register{overflow-x:auto}\
        table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}\
        caption.visually-hidden{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}\
        th{padding:14px 0 9px;border-bottom:1px solid var(--rule-strong);text-align:left;\
        font-family:var(--mono);font-size:11px;font-weight:500;letter-spacing:.12em;\
        text-transform:uppercase;color:var(--text-dim);white-space:nowrap}\
        td{padding:13px 0 13px 0;border-bottom:1px solid var(--rule);vertical-align:baseline;white-space:nowrap}\
        td+td,th+th{padding-left:18px}\
        tr.row:hover td{background:var(--row-hover)}\
        .col-time{width:96px;font-family:var(--mono);font-size:13px;letter-spacing:.03em}\
        .col-device{width:auto;min-width:240px;white-space:normal}\
        .col-addr{text-align:right;font-family:var(--mono);font-size:12.5px;color:var(--text-dim)}\
        .col-first{width:120px;text-align:right;font-family:var(--mono);font-size:12.5px;color:var(--text-dim)}\
        .col-state{width:120px;text-align:right}\
        .device-name{font-size:15px;font-weight:600}\
        .device-name.no-name{font-weight:400;font-style:italic;color:var(--text-dim)}\
        .device-mac{display:block;margin-top:3px;font-family:var(--mono);font-size:11px;\
        letter-spacing:.04em;color:var(--text-dim)}\
        .device-addr{display:none}\
        .flag{display:inline-block;margin-left:8px;padding:1px 6px;border:1px solid var(--amber);\
        color:var(--amber);font-family:var(--mono);font-size:11px;letter-spacing:.1em;\
        text-transform:uppercase;vertical-align:1px}\
        .flap{display:inline-block;padding:4px 10px;background-color:var(--flap-bg);color:var(--flap-ink);\
        background-image:linear-gradient(transparent 48%,var(--seam) 48%,var(--seam) 52%,transparent 52%);\
        font-family:var(--mono);font-size:11px;letter-spacing:.12em;text-transform:uppercase}\
        .flap-departed{background-color:transparent;background-image:none;color:var(--flap-dep-ink);\
        border:1px solid var(--rule-strong)}\
        tr.departed .device-name,tr.departed .col-addr,tr.departed .col-time{color:var(--text-dim);font-weight:500}\
        tr.divider td{padding:22px 0 8px;border-bottom:1px solid var(--rule-strong);\
        font-family:var(--mono);font-size:11px;letter-spacing:.14em;text-transform:uppercase;color:var(--text-dim)}\
        .empty{padding:56px 0 40px;max-width:52ch}\
        .empty h2{margin:0;font-size:20px;font-weight:650;letter-spacing:-.01em}\
        .empty p{margin:10px 0 0;color:var(--text-dim);font-size:14px}\
        .empty-action a{color:var(--text);text-decoration:underline;text-underline-offset:3px}\
        .colophon{margin-top:26px;font-family:var(--mono);font-size:11px;letter-spacing:.02em;color:var(--text-dim)}\
        .colophon p{margin:0}\
        @media (max-width:720px){\
        .board{padding:18px 16px 30px}\
        .clock-value{font-size:20px}\
        .figures{grid-template-columns:repeat(2,1fr)}\
        .figure:nth-child(2){border-right:0}\
        .figure:nth-child(-n+2){border-bottom:1px solid var(--rule)}\
        .figure b{font-size:30px}\
        .col-first{display:none}\
        .col-addr{display:none}\
        .device-addr{display:block;margin-top:2px;font-family:var(--mono);font-size:11px;color:var(--text-dim)}\
        table{min-width:0}\
        .col-device{min-width:0}\
        .col-time{width:86px}\
        .col-state{width:86px}\
        th{font-size:11px;letter-spacing:.1em}\
        td{padding:11px 0}\
        .filters{width:100%}\
        .key{flex:1 1 auto;text-align:center;padding:7px 8px;font-size:11px}\
        .device-mac{font-size:11px}}"
    }
}

/// A row's age in the register's own notation: NOW for a device in the last
/// scan, then rounded minutes, hours and days. Deliberately terse — this
/// column is scanned, not read.
fn board_age(epoch_seconds: u64, now: u64, present: bool) -> String {
    if present {
        return "NOW".to_string();
    }
    let age = now.saturating_sub(epoch_seconds);
    if age < 3_600 {
        return format!("{} M", (age / 60).max(1));
    }
    if age < 86_400 {
        return format!("{} H", (age / 3_600).max(1));
    }
    format!("{} D", age / 86_400)
}

/// The scan age in the status strip: seconds while they are still legible as
/// seconds, then minutes, hours and days. Never "0 minutes ago".
fn scan_age(age_seconds: u64) -> String {
    if age_seconds < 5 {
        return "just now".to_string();
    }
    if age_seconds < 60 {
        return format!("{age_seconds} s ago");
    }
    let minutes = age_seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("{hours} h ago");
    }
    format!("{} d ago", hours / 24)
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
    use crate::model::now_epoch_seconds;
    use crate::scan::{ScanContext, run_scan};
    use axum::body::to_bytes;
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn observation_with_hostname(mac: &str, ip: &str, hostname: &str) -> Observation {
        Observation {
            mac: model::Mac::new(mac),
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

    /// Build a `ScanContext` scanning the loopback interface over `db` and
    /// `discovery`, cloning the shared handles so the caller keeps using them.
    fn scan_ctx(db: &Arc<Db>, discovery: &Arc<dyn Discovery + Send + Sync>) -> ScanContext {
        ScanContext {
            db: db.clone(),
            discovery: discovery.clone(),
            interface: "lo".to_string(),
        }
    }

    async fn render_table(state: Arc<AppState>) -> String {
        render_path(state, "/table").await
    }

    /// Render any route, so page-level and fragment-level behaviour can be
    /// checked through the real router.
    async fn render_path(state: Arc<AppState>, uri: &str) -> String {
        let app = router().with_state(state);
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

    /// Two devices, one in the latest scan and one only seen before it.
    async fn db_with_present_and_departed(name: &str) -> Arc<Db> {
        let db = open_temp_db(name);
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:01"),
            "10.0.0.1",
            Some("on.lan"),
            now,
        )
        .expect("insert succeeds");
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:02"),
            "10.0.0.2",
            Some("off.lan"),
            now - 3_600,
        )
        .expect("insert succeeds");
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "on.lan",
            )]]));
        run_scan(&scan_ctx(&db, &discovery), now)
            .await
            .expect("scan succeeds");
        db
    }

    #[tokio::test]
    async fn filters_narrow_the_register_without_hiding_the_way_back() {
        let db = db_with_present_and_departed("lanlord.test.web.filters.sqlite").await;
        let present = render_path(state_with(db.clone()), "/?filter=present").await;
        assert!(
            present.contains("on.lan"),
            "present slice keeps live rows: {present}"
        );
        assert!(
            !present.contains("off.lan"),
            "present slice drops departed rows"
        );
        assert!(
            present.contains("aria-current=\"page\""),
            "the current slice is marked for assistive technology: {present}"
        );

        let departed = render_path(state_with(db.clone()), "/?filter=departed").await;
        assert!(
            departed.contains("off.lan"),
            "departed slice keeps gone rows: {departed}"
        );
        assert!(
            !departed.contains("on.lan"),
            "departed slice drops live rows"
        );

        let all = render_path(state_with(db), "/").await;
        assert!(
            all.contains("Departed · 1"),
            "the full register cuts the departed group off with one rule: {all}"
        );
        assert!(all.contains("on.lan") && all.contains("off.lan"));
    }

    #[tokio::test]
    async fn an_empty_slice_says_what_is_true_and_offers_the_full_register() {
        let db = open_temp_db("lanlord.test.web.empty-slice.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:01"),
            "10.0.0.1",
            Some("on.lan"),
            now,
        )
        .expect("insert succeeds");
        let discovery: Arc<dyn Discovery + Send + Sync> =
            Arc::new(Scripted::new(vec![vec![observation_with_hostname(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                "on.lan",
            )]]));
        run_scan(&scan_ctx(&db, &discovery), now)
            .await
            .expect("scan succeeds");

        let body = render_path(state_with(db), "/?filter=departed").await;
        assert!(
            body.contains("Nothing has departed"),
            "empty slice is explained: {body}"
        );
        assert!(
            body.contains("href=\"/?filter=all\""),
            "the empty slice offers the full register: {body}"
        );
        assert!(!body.contains("<table"), "no empty table skeleton is drawn");
    }

    #[test]
    fn board_age_speaks_in_the_register_notation() {
        let now = 1_700_000_000;
        assert_eq!(board_age(now, now, true), "NOW");
        assert_eq!(board_age(now - 20, now, false), "1 M");
        assert_eq!(board_age(now - 600, now, false), "10 M");
        assert_eq!(board_age(now - 7_200, now, false), "2 H");
        assert_eq!(board_age(now - 2 * 86_400, now, false), "2 D");
    }

    #[test]
    fn scan_age_never_reads_zero_minutes() {
        assert_eq!(scan_age(0), "just now");
        assert_eq!(scan_age(45), "45 s ago");
        assert_eq!(scan_age(60), "1 min ago");
        assert_eq!(scan_age(3_600), "1 h ago");
        assert_eq!(scan_age(7_200), "2 h ago");
        assert_eq!(scan_age(3 * 86_400), "3 d ago");
    }

    #[tokio::test]
    async fn table_renders_hostname_value_when_present() {
        let db = open_temp_db("lanlord.test.web.hostname-present.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:01"),
            "10.0.0.1",
            Some("gateway.home.arpa"),
            now,
        )
        .expect("insert succeeds");
        let body = render_table(state_with(db)).await;
        assert!(
            body.contains("<th class=\"col-device\">Device</th>"),
            "the register has a Device column: {body}"
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
        db.upsert_device(&model::Mac::new("aa:bb:cc:dd:ee:01"), "10.0.0.1", None, now)
            .expect("insert succeeds");
        let body = render_table(state_with(db)).await;
        assert!(
            body.contains("<th class=\"col-device\">Device</th>"),
            "the register has a Device column: {body}"
        );
        assert!(
            body.contains("no hostname"),
            "unknown hostname is said in words, not a dash: {body}"
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
        run_scan(&scan_ctx(&db, &discovery), now_epoch_seconds())
            .await
            .expect("first scan succeeds");
        let before = render_table(state_with(db.clone())).await;
        assert!(
            before.contains("old.home.arpa"),
            "first hostname renders: {before}"
        );
        assert!(!before.contains("new.home.arpa"));

        run_scan(&scan_ctx(&db, &discovery), now_epoch_seconds())
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
