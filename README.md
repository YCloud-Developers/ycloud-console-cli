# yc - YCloud Console CLI

`yc` is the Console/Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `yc login` uses Dashboard browser grant + PKCE, receives the code through a localhost callback by default, and stores `YCLI.` tokens.
- `yc whoami` reads the current Console CLI identity.
- `yc tenants list` lists tenants available to the current Console CLI token.
- `yc analytics ...` calls the same Dashboard analytics APIs used by `/app/dashboard/analytics`.
- `yc refresh` rotates the refresh token.
- `yc logout` revokes the current token and removes the local profile.

Manual copy-paste authorization code input is still available with `yc login --manual` for terminals that cannot receive a localhost browser callback.

## Local Test Against Dashboard

Start account-service, security, and web with the same route group:

```bash
-Dqipeng.client.group=sqj -Dqipeng.server.group=sqj
```

Then run:

```bash
cargo run -- login --dashboard-url http://127.0.0.1:8036 --scope developers
```

The CLI opens a browser and waits for Dashboard to redirect back to `http://127.0.0.1:<port>/callback`.

If automatic browser login is not available, use the manual fallback:

```bash
cargo run -- login --dashboard-url http://127.0.0.1:8036 --scope developers --manual
```

In manual mode, open the printed URL in a browser that is already logged in to Dashboard, copy the returned `data.code`, then paste it into the CLI prompt.

## Commands

```bash
yc login --dashboard-url http://127.0.0.1:8036 --scope developers
yc whoami
yc tenants list
yc analytics outline
yc analytics overview
yc analytics logs --page-no 1 --page-size 20
yc analytics calling-logs --page-no 1 --page-size 20
yc refresh
yc logout
```

For dev-blue analytics testing:

```bash
cargo run -- login --dashboard-url https://www-dev-blue.ycloud.com --scope "developers whatsapp:manager:analytics"
cargo run -- analytics outline
cargo run -- analytics overview --timezone GMT+8
cargo run -- analytics logs --page-no 1 --page-size 10
cargo run -- analytics calling-logs --page-no 1 --page-size 10
```

Analytics commands default to the last 7 days. Use millisecond timestamps to pin the same range as the Dashboard page:

```bash
yc analytics overview --start-time 1782921600000 --end-time 1783526400000 --from 8613800138000 --region-code CN --message-category marketing,utility
yc analytics logs --start-time 1782921600000 --end-time 1783526400000 --direction OutBound --status sent,delivered --source "WhatsApp Business API"
yc analytics calling-logs --start-time 1782921600000 --end-time 1783526400000 --directions BUSINESS_INITIATED --sources CALLING --status COMPLETED
```

## Config

Config is stored at `~/.yc/config.toml`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.
