# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

One homelab operator running lanlord on their own LAN, on a host they control
(typically NixOS). They are technical, self-host the surrounding stack, and
visit the UI briefly to answer a concrete question: which devices are on the
segment, and have known devices come or gone. lanlord is a personal utility,
not a shared team dashboard; the instance may be reachable by others on the
LAN or VPN, but its operator is the person who deployed and maintains it.

## Product Purpose

lanlord inventories the devices on a single home-network segment. It ARP-scans
the segment on a fixed interval, persists every device it has ever seen, and
shows the result in a small read-only web UI. Success means the operator can
tell at a glance what is present, what has appeared recently, and when each
device was first and last seen — without maintaining a spreadsheet or trusting
a live-only scan that forgets anything absent.

## Positioning

A single-segment, MAC-keyed inventory that remembers history rather than an
IP-keyed live view. Neighboring tools either show only the current scan or
require a heavier collector/agent model; lanlord is one Rust binary with an
embedded web UI, deployed from Nix, with no frontend build toolchain, no
JavaScript framework, and no authentication layer to configure.

## Operating Context

- Configured by a TOML file passed with `--config` (`interface` required;
  `subnet`, `scan_interval`, `listen`, `database` optional), with
  `--listen`/`--database` CLI overrides.
- Deployed via `nix run`, from a dev shell, or through the NixOS module
  (dedicated `lanlord` user, `CAP_NET_RAW` only, firewall closed by default).
- Runs against a trusted LAN or VPN; anything beyond that is expected to sit
  behind a reverse proxy that terminates TLS and enforces access control.
- Inventory persists in SQLite under `stateDir`; backups are the operator's
  responsibility, with the documented WAL caveat.
- Success is verified by watching the service journal and the device table
  on a physical LAN, since CI cannot reach one.

## Capabilities and Constraints

- **Scan**: one complete ARP sweep of one IPv4 segment on a configurable
  interval (default 300 s).
- **Inventory**: every device ever seen, including ones absent from the most
  recent scan; devices keyed by MAC, with IP as only the latest observed
  address and hostname as optional.
- **Web UI**: Axum + Maud server-rendered HTML; HTMX polls the device table
  every 10 s; read-only, with known/present/departed counts, arrivals in the
  last 24 h, relative first/last-seen times, and server-side filter links for
  all, present, departed, and recent arrivals.
- Permanent v1 boundaries: one segment only (devices behind a router are
  out of reach), MAC identity (never IP), no authentication, no write
  operations from the UI.
- No JavaScript build step and no external asset/CDN requests; HTMX is
  vendored and inlined.
- Beyond the device table, product scope is undecided.

## Brand Commitments

The name **lanlord** is fixed. The voice is plain, direct technical prose with
no marketing claims. No logo, mascot, tagline, or personality assets exist or
are required.

## Evidence on Hand

- `README.md`: what lanlord does and its boundaries, trial paths,
  configuration, NixOS module, exposure, backup, and a manual smoke test.
- `GLOSSARY.md`: canonical domain vocabulary (Device, Inventory, Presence,
  First seen / Last seen, Segment, Scan, scan interval) and terms to avoid.
- `docs/adr/0001-mac-is-device-identity.md` and
  `docs/adr/0002-server-rendered-maud-htmx.md`.
- Tests covering the database, empty inventory, HTMX refresh, presence, and
  hostname rendering.

There are no customers, testimonials, benchmarks, pricing, press, or brand
assets. Future work must not fabricate them.

## Product Principles

1. **History is the product.** An inventory that forgets absent devices is a
   live view, not an inventory; presence is a property of the last scan, not a
   replacement for the record.
2. **Identity is MAC.** IP addresses are observations, not identity; anything
   that reshuffles the Inventory on a DHCP lease change is wrong.
3. **One segment, honestly bounded.** Do not imply visibility into routed
   networks or invent fallbacks that pretend to.
4. **Operate from evidence.** Show when the last scan ran and when the view
   was refreshed; stale or failed scans must not look like a healthy empty
   network.
5. **Boring to deploy and audit.** A single binary, a pinned Nix toolchain, no
   frontend build step, and no silent changes to network exposure.
6. **Speak the glossary.** Device, Inventory, Presence, Segment, Scan, and
   scan interval mean what `GLOSSARY.md` says; avoid its listed synonyms.
