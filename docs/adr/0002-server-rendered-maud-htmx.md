# Server-rendered HTML with Maud and HTMX

lanlord's backend and frontend are one Rust binary: Axum serves HTML rendered with Maud macros, with HTMX for the small amount of interactivity (polling the device table). There is no JavaScript build step and no WASM toolchain to pin in Nix.

Considered and rejected for v1: Leptos SSR, which puts real Rust components in the browser but drags in `wasm32`, `cargo-leptos`, and `wasm-bindgen` version matching; and a CSR SPA over a JSON API, which is the slowest path to a read-only table. If the UI later outgrows server-rendered fragments, moving to WASM is a frontend change, not a domain change.
