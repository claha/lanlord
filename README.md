# lanlord

Self-hosted LAN device inventory for homelabs. It ARP-scans one L2 segment,
remembers every device it has seen, and shows them in a small web UI.

Early development — the table of devices is the v1 product.

## Development

```sh
nix develop
cargo run
```

## Checks

CI runs these four commands on every pull request and on every push to `main`,
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
