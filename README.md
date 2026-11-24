# lanlord

Self-hosted LAN device inventory for homelabs. lanlord ARP-scans one L2
segment, remembers every device it has ever seen, and shows them in a small
web UI. Built and deployed with Nix; a single Rust binary, no frontend
build toolchain.

Early development — the device table is the v1 product.

## What lanlord does — and its boundaries

lanlord is a single-segment LAN inventory. It does three things:

1. **Scan**: one complete ARP sweep of the segment, producing the set of
   devices currently present.
2. **Inventory**: the persisted set of every device ever seen, including ones
   absent from the current scan. Each device is keyed by its MAC address;
   its IP is only the most recent address observed.
3. **Web UI**: Axum serves HTML (rendered with Maud) with HTMX polling the
   device table, on one TCP listen address.

Boundaries (these are permanent v1 design decisions, not omissions):

* **One Segment.** Scanning is one IPv4 subnet reachable by ARP from a single
  interface. Devices behind a router can never be inventoried. See
  [MAC address is the device identity](docs/adr/0001-mac-is-device-identity.md).
* **MAC identity.** A device's identity is its MAC address, never its IP.
* **No authentication.** lanlord has no auth of any kind. See
  [Exposure](#exposure) below.

Terminology follows [GLOSSARY.md](GLOSSARY.md) (Device, Inventory, Presence,
Segment, Scan, scan interval). The UI design is recorded in
[docs/adr/0002-server-rendered-maud-htmx.md](docs/adr/0002-server-rendered-maud-htmx.md).

## Try it

Two trial paths, both from a clean checkout with no undocumented steps:

### With `nix run` (no dev shell needed)

```sh
# Pick the interface to scan, e.g. eth0, enp3s0, wlan0.
cat > lanlord.toml <<'EOF'
interface = "eth0"
EOF

nix run . -- --config lanlord.toml
```

`nix run .` builds and runs the `lanlord` binary from this flake. It binds
to `127.0.0.1:8080` by default, so open `http://127.0.0.1:8080` on the same
host.

### With `cargo run` from the dev shell

```sh
nix develop
cargo run -- --config lanlord.toml
```

The dev shell (`nix develop`) provides `cargo`, `rustc`, `clippy`, `rustfmt`,
`pkg-config`, and `sqlite` from `flake.lock`, so CI and your machine use the
identical toolchain.

Both paths require `--config`; see [Configuration](#configuration) below.

## Configuration

lanlord is configured by a TOML file passed with `--config`. `interface` is
required; everything else has a default.

### Complete TOML example

```toml
# lanlord.toml — one Segment per config file
interface = "eth0"

# Optional: override the segment derived from the interface's IPv4 address.
subnet = "10.0.0.0/24"

# Seconds between the starts of consecutive scans (default 300).
scan_interval = 300

# Address the web UI listens on (default 127.0.0.1:8080).
listen = "127.0.0.1:8080"

# Path to the SQLite inventory database
# (default /var/lib/lanlord/lanlord.db).
database = "/var/lib/lanlord/lanlord.db"
```

### CLI overrides

`--config` is mandatory; lanlord refuses to start without it:

```sh
nix run . -- --config lanlord.toml
```

`--listen` and `--database` override the values in the config file:

```sh
nix run . -- --config lanlord.toml --listen 0.0.0.0:8080 --database /data/lanlord.db
```

| Flag | Overrides | Default |
|------|-----------|---------|
| `--config` | (required, no default) | — |
| `--listen` | `listen` | `127.0.0.1:8080` |
| `--database` | `database` | `/var/lib/lanlord/lanlord.db` |

`interface` has no CLI override; set it in the TOML file. The scanned segment
is derived from the interface's IPv4 address unless `subnet` is set.

## Deploy with the NixOS module

Enable the service via the [NixOS module](modules/lanlord.nix). Copy-pasteable
snippet:

```nix
{ config, pkgs, lib, ... }:

{
  # Import the module (as a flake input):
  # modules = [ inputs.lanlord.nixosModules.default ];

  lanlord.enable = true;
  lanlord.package = pkgs.lanlord;  # or inputs.lanlord.packages.x86_64-linux.default

  # Which interface to ARP-scan (required when enabled).
  lanlord.interface = "eth0";

  # Optional segment override; defaults to the interface's IPv4 segment.
  # lanlord.subnet = "10.0.0.0/24";

  # Seconds between scans (default 300).
  lanlord.scanInterval = 300;

  # Address the web UI listens on (default 127.0.0.1:8080).
  # Use a loopback address if you front it with a reverse proxy — see Exposure.
  lanlord.listenAddress = "127.0.0.1:8080";

  # Directory holding the SQLite database (default /var/lib/lanlord).
  lanlord.stateDir = "/var/lib/lanlord";

  # Open the host firewall for the listen port (default false).
  # Disabled by design: enabling the service never silently changes exposure.
  lanlord.openFirewall = false;
}
```

Module options referenced: `enable`, `interface`, `subnet`, `scanInterval`,
`listenAddress`, `stateDir`, `openFirewall`. The module runs as a dedicated
`lanlord` system user with only `CAP_NET_RAW` (for ARP), so the service
itself is hardened; see `modules/lanlord.nix` for the full sandbox.

## Exposure

lanlord has **no authentication**. Anyone who can reach the listen address
can read the full inventory.

* On a trusted LAN/VPN, binding to `0.0.0.0:8080` (and opening the firewall)
  is a deliberate operator choice — set `lanlord.openFirewall = true` in the
  NixOS module.
* For anything beyond LAN/VPN, do **not** expose lanlord directly. Front it
  with a reverse proxy (nginx, Caddy, Traefik, ...) terminating TLS and
  enforcing access control, and keep lanlord bound to loopback
  (`127.0.0.1:8080`). The reverse proxy is your auth and TLS boundary,
  because lanlord itself provides neither.

## Backup

The inventory lives in the SQLite database inside `stateDir`
(default `/var/lib/lanlord`, NixOS module default `/var/lib/lanlord`).

To back up, copy the whole state directory:

```sh
# NixOS module default:
sudo cp -a /var/lib/lanlord /mnt/backups/lanlord-$(date +%F).dir
```

**WAL caveat.** SQLite writes to a write-ahead log (`lanlord.db-wal`)
alongside the main file, and a checkpoint merges it back into `lanlord.db`.
Copying files while the service is running can capture a `lanlord.db` whose
WAL has not been checkpointed, so the copy may miss recent rows.

For a consistent copy either:

1. Stop the service first (`sudo systemctl stop lanlord`), then copy; or
2. Use SQLite's online backup API (a proper backup tool) instead of raw file
   copies.

Restore by replacing the state directory contents and restarting the service.

## Manual smoke test on a physical LAN

CI cannot reach a physical LAN, so this is a manual procedure. Do it on the
host that will run lanlord, with a real ARP-reachable interface.

1. Pick an interface with an IPv4 address, e.g. `eth0`.
   ```sh
   ip -brief addr
   ```
2. Write a config that scans it:
   ```sh
   cat > lanlord.toml <<'EOF'
   interface = "eth0"
   listen = "0.0.0.0:8080"
   EOF
   ```
3. Start lanlord (one of the trial paths):
   ```sh
   nix run . -- --config lanlord.toml
   ```
4. **Expected terminal/journal output** on success, in order:
   ```
   lanlord: listening on 0.0.0.0:8080
   ```
   A startup scan runs before serving; if it fails you instead see
   `lanlord: startup scan failed: <cause>` and the UI serves the last known
   inventory (empty on first run). The process keeps running either way.
5. Open `http://<this-host-ip>:8080` from another machine on the same LAN.
   The device table lists the hosts that answered the startup ARP scan,
   keyed by MAC, with first/last seen timestamps.
6. Wait at least one `scan_interval` (default 300 s) or force a refresh, and
   confirm the table still shows devices and that Presence updates as hosts
   come and go.
7. Stop lanlord with `Ctrl-C`.

When running under the NixOS module instead, watch the journal while starting:

```sh
sudo journalctl -u lanlord -f
```

Expected output, in order:
```
lanlord: listening on 127.0.0.1:8080
```
and, only when the startup scan cannot run,
```
lanlord: startup scan failed: <cause>
```

## Documentation links

* [GLOSSARY.md](GLOSSARY.md) — Device, Inventory, Presence, Segment, Scan,
  scan interval, and the vocabulary to avoid.
* [docs/adr/0001-mac-is-device-identity.md](docs/adr/0001-mac-is-device-identity.md)
  — why MAC is the device identity and the one-segment boundary.
* [docs/adr/0002-server-rendered-maud-htmx.md](docs/adr/0002-server-rendered-maud-htmx.md)
  — why the UI is server-rendered Maud + HTMX with no JavaScript build.

## Checks

CI runs these four commands on every pull request and every push to `main`,
inside the flake dev shell so CI and local use the identical toolchain from
`flake.lock`:

```sh
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
nix build
```

## License

MIT
