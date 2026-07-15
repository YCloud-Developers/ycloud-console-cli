# Repository Guidelines

## Project Scope

This is the Console/Dashboard-oriented YCloud CLI. It is separate from the existing OpenAPI/API-key `ycloud-cli` project and must not reuse `~/.ycli/config.toml` or API-key authentication.

## Build and Test

- `cargo fmt -- --check` checks formatting.
- `cargo test` runs unit and integration tests.
- `cargo run -- login --dashboard-url http://127.0.0.1:8036 --profile readonly` runs the local browser-grant flow.

## Auth Model

- `yc login` uses Dashboard browser grant plus PKCE.
- CLI grants use stable atomic permissions such as `yc.contact.record.read`. `--profile` selects a backend-owned preset and repeatable `--permission` adds atoms; Dashboard role authorities remain an internal backend mapping only.
- Access tokens use the backend `YCLI.` token prefix.
- Local config is stored under `~/.yc/config.toml`.
- Do not log or print access tokens, refresh tokens, authorization codes, or Dashboard cookies.

## Coding Style

Use Rust 2021, `clap` for commands, `reqwest` for HTTP, and strongly typed request/response structs. Keep command handlers thin and move protocol/config logic into modules with tests.
