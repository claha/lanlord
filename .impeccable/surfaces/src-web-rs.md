---
version: 1
slug: "src-web-rs"
primary_target: "src/web.rs"
related_targets: []
---

# Surface brief: lanlord device screen (`/`)

Scope: the single read-only inventory page and its HTMX table fragment
(`/table`). Mode: **Operate**.

## Audience and job

The homelab operator, technical, self-hosting, visiting briefly. Their
question, confirmed in the ask round: **what changed since I last looked** —
arrivals, departures, and how stale each fact is. Secondary: the full
MAC-keyed record. Action: read, filter, leave. No destructive actions exist.
Constraints: one Rust binary, Maud markup, vendored HTMX only, no external
asset requests, no auth, both OS appearances handled, phone-width readable,
the three integration test files and two unit tests keep their intent.

Proof/content: the persisted inventory itself — hostname, MAC, address,
first seen, last seen, presence derived from the last scan start, and the
24-hour arrival count. No invented claims; synthetic data only for review
captures.

## Direction contract

**THESIS:** A live departures board for the segment, where last-seen time
leads every row and state is a flap — refusing the dashboard's
summary-cards-plus-table default.

**OWN-WORLD:** Enamel near-black ground, warm off-white cells, amber only for
the clock and the arrival flag; one condensed caps voice for devices, one
tabular mono for times and addresses; hairline seams, no radii, no shadows
except the board's inset edge. Daylight appearance flips ground and cells
and keeps amber. Type: one tracked caps voice for labels, figures and
flaps; device names stay sentence case in the system sans, because
uppercasing data trades clarity for costume; one tabular mono for times,
addresses and MACs.

**STORY:** The operator reads, in one scan, what arrived, what left, and how
stale the information is; departure is a place on the page, not a filter
hiding it.

**FIRST VIEWPORT:** Wordmark and segment line left with the amber next-poll
clock right; four inline figures (known, present, departed, arrivals in
24 h); a filter key row; then the register, each row led by its last-seen
time (NOW / 03:14 / 2 D) and closed by a state flap.

**FORM:** Position 1 of 7 ordered directions (Departure Board); seed:
seed:a89f183734ea2ffb.

**FINISH:** unreviewed and undocumented is unfinished; this build ends with
the finish review, the verdict, DESIGN.md, and every shipping raster carrying
its provenance.

## Unresolved

- Whether the light appearance ships as an equal-quality daylight board or as
  a reduced fallback; the build should treat it as equal.
- Copy for a filtered empty state (departed filter with nothing departed) is
  drafted in the build, not yet reviewed by the user.
