# Repository Guidelines

## Project Scope

This is the Console/Dashboard-oriented YCloud CLI. It is separate from the existing OpenAPI/API-key `ycloud-cli` project and must not reuse `~/.ycli/config.toml` or API-key authentication.

## Build and Test

- `cargo fmt -- --check` checks formatting.
- `cargo test` runs unit and integration tests.
- `YCLOUD_OAUTH_TEST_LOOPBACK=1 cargo run -- login --dashboard-url http://127.0.0.1:8036 --oauth-issuer http://127.0.0.1:8036 --profile readonly` runs standard OAuth against a local mock.

## Auth Model

- `ycloud login` always uses standard OAuth discovery, PKCE S256, a bound CLI resource and issuer-validated `/oauth/callback`. Production business and issuer default to `https://open-platform.ycloud.com`; a new explicit login binds the logical CLI resource returned by trusted discovery; a custom Dashboard origin requires explicit `--oauth-issuer`. Hidden `--oauth` is an accepted no-op for existing scripts.
- CLI grants use stable atomic permissions such as `yc.contact.record.read`. `--profile` selects a backend-owned preset and repeatable `--permission` adds atoms; Dashboard role authorities remain an internal backend mapping only.
- Standard OAuth tokens are opaque. Saved `auth.oauth` bindings select standard refresh/revoke; profiles without bindings are rejected before HTTP with a new-login prompt. Never reinterpret legacy tokens or fall back to `/api/cli/auth/**`. Retired Web business endpoints return410; legacy auth is not a fallback.
- Standard sessions persist discovered token/revoke endpoints and the business origin. Validate bindings before HTTP, disable redirects/proxies, and never replay an unconfirmed refresh. Loopback HTTP requires `YCLOUD_OAUTH_TEST_LOOPBACK=1`.
- Local config is stored under `~/.ycloud/config.toml`.
- Do not create or open profile lock files. Read-only queries must not write config or migrate trial profiles. Credential writes keep atomic saves and the unknown-refresh marker, but do not promise cross-process serialization; server replay protection can reject concurrent refreshes and revoke the grant.
- Do not log or print access tokens, refresh tokens, authorization codes, or Dashboard cookies.

## Coding Style

Use Rust 2021, `clap` for commands, `reqwest` for HTTP, and strongly typed request/response structs. Keep command handlers thin and move protocol/config logic into modules with tests.
