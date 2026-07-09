# yc - YCloud Dashboard CLI

`yc` is the Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `yc login` uses Dashboard browser grant + PKCE and stores `YCLI.` tokens.
- `yc whoami` reads the current Dashboard CLI identity.
- `yc tenants list` lists tenants available to the current Dashboard CLI token.
- `yc refresh` rotates the refresh token.
- `yc logout` revokes the current token and removes the local profile.

Current backend behavior returns an authorization code as JSON from `/api/cli/auth/authorize`, so the first implementation uses copy-paste authorization code input.

## Local Test Against Dashboard

Start account-service, security, and web with the same route group:

```bash
-Dqipeng.client.group=sqj -Dqipeng.server.group=sqj
```

Then run:

```bash
cargo run -- login --dashboard-url http://127.0.0.1:8036 --scope developers
```

Open the printed URL in a browser that is already logged in to Dashboard, copy the returned `data.code`, then paste it into the CLI prompt.

## Commands

```bash
yc login --dashboard-url http://127.0.0.1:8036 --scope developers
yc whoami
yc tenants list
yc refresh
yc logout
```

## Config

Config is stored at `~/.yc/config.toml`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.
