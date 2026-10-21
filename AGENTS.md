# Repository Guidelines

## Project Scope

This is the Console/Dashboard-oriented YCloud CLI. It is separate from the existing OpenAPI/API-key `ycloud-cli` project and must not reuse `~/.ycli/config.toml` or API-key authentication.

## Build and Test

- `cargo fmt -- --check` checks formatting.
- `cargo test` runs unit and integration tests.
- `cargo run -- login --dashboard-url http://127.0.0.1:8036 --profile readonly` runs the local browser-grant flow.

## Auth Model

- Default `ycloud login` uses the legacy Dashboard browser grant plus PKCE; opt-in `login --oauth` uses standard discovery, PKCE S256, a bound CLI resource and issuer-validated `/oauth/callback`.
- CLI grants use stable atomic permissions such as `yc.contact.record.read`. `--profile` selects a backend-owned preset and repeatable `--permission` adds atoms; Dashboard role authorities remain an internal backend mapping only.
- Legacy access tokens use the backend `YCLI.` token prefix. Standard OAuth tokens are opaque and are never parsed as legacy tokens. Saved `auth.oauth` bindings select refresh/revoke; absent bindings mean legacy, without protocol fallback.
- Standard sessions persist discovered token/revoke endpoints and the business origin. Validate bindings before HTTP, disable redirects/proxies, and never replay an unconfirmed refresh. Loopback HTTP requires `YCLOUD_OAUTH_TEST_LOOPBACK=1`.
- Local config is stored under `~/.ycloud/config.toml`.
- Do not create or open profile lock files. Read-only queries must not write config or migrate trial profiles. Credential writes keep atomic saves and the unknown-refresh marker, but do not promise cross-process serialization; server replay protection can reject concurrent refreshes and revoke the grant.
- Do not log or print access tokens, refresh tokens, authorization codes, or Dashboard cookies.

## Coding Style

Use Rust 2021, `clap` for commands, `reqwest` for HTTP, and strongly typed request/response structs. Keep command handlers thin and move protocol/config logic into modules with tests.
