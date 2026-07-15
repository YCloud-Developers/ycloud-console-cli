# ycloud - YCloud Console CLI

`ycloud` is the Console/Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `ycloud login` uses Dashboard browser grant + PKCE, receives the code through a localhost callback by default, and stores `YCLI.` tokens.
- `ycloud whoami` reads the current Console CLI identity.
- `ycloud tenants list` lists tenants available to the current Console CLI token.
- `ycloud integrations status` lists Dashboard integration enabled status through the CLI read-only adapter.
- `ycloud contacts metadata` lists contact sources, tags, segments, and segment filters through the CLI read-only adapter.
- `ycloud contacts list` and `ycloud analytics ...` use permission-gated `/api/cli/read/**` compatibility adapters; `YCLI.` tokens do not call ordinary Dashboard paths.
- `ycloud refresh` rotates the refresh token.
- `ycloud logout` revokes the current token and removes the local profile.

Manual copy-paste authorization code input is still available with `ycloud login --manual` for terminals that cannot receive a localhost browser callback.

## Installation

After the first public release, install the CLI with npm:

```bash
npm install --global @ycloud-ai/console-cli
ycloud --version
```

Or install it from the YCloud Homebrew tap:

```bash
brew install YCloud-Developers/tap/ycloud
ycloud --version
```

Release packages support macOS and Linux on ARM64 and x64.

## Local Test Against Dashboard

Run the required Dashboard services locally, then pass the local URL explicitly:

```bash
cargo run -- login --dashboard-url http://127.0.0.1:8036 --profile readonly
```

The CLI opens a browser and waits for Dashboard to redirect back to `http://127.0.0.1:<port>/callback`.

If automatic browser login is not available, use the manual fallback:

```bash
cargo run -- login --dashboard-url http://127.0.0.1:8036 --profile readonly --manual
```

In manual mode, open the printed URL in a browser that is already logged in to Dashboard, copy the returned `data.code`, then paste it into the CLI prompt.

## Commands

```bash
ycloud login --profile readonly
ycloud whoami
ycloud tenants list
ycloud integrations status
ycloud contacts metadata
ycloud analytics outline
ycloud analytics overview
ycloud analytics logs --page-no 1 --page-size 20
ycloud analytics calling-logs --page-no 1 --page-size 20
ycloud refresh
ycloud logout
```

Analytics commands default to the last 7 days. Use millisecond timestamps to pin the same range as the Dashboard page:

```bash
ycloud analytics overview --start-time 1782921600000 --end-time 1783526400000 --from 8613800138000 --region-code CN --message-category marketing,utility
ycloud analytics logs --start-time 1782921600000 --end-time 1783526400000 --direction OutBound --status sent,delivered --source "WhatsApp Business API"
ycloud analytics calling-logs --start-time 1782921600000 --end-time 1783526400000 --directions BUSINESS_INITIATED --sources CALLING --status COMPLETED
```

## Online Smoke

The P0 read-only Dashboard CLI commands are merged into `main` and can be tested against online after `ycloud login`:

```bash
ycloud whoami
ycloud integrations status
ycloud contacts metadata
```

The expected result is:

- `whoami` returns the current Dashboard CLI user, tenant, and granted permissions.
- `integrations status` returns enabled Dashboard integrations.
- `contacts metadata` returns contact `segmentFilters`, `segments`, `sources`, and `tags`.

## Permission Profiles

`ycloud login` defaults to `--profile basic`. Available profiles are `basic`, `contacts-read`, `analytics-read`, `integrations-read`, `readonly`, and `custom`. Add individual ACTIVE permissions by repeating `--permission`:

```bash
ycloud login --profile basic \
  --permission yc.integration.status.read \
  --permission yc.contact.record.read
```

Profiles are expanded by the backend. The token and local config store only the resulting atomic requested permissions. PLANNED permissions, including all currently catalogued writes, cannot be requested. HTTP requests have a 30-second total timeout.

In the default browser flow, an authorization rejection returns to the localhost callback with `error`, `error_description`, and `state`. The CLI validates `state` before reporting the rejection, exits immediately, and does not exchange a token or write the profile.

## Config

Config is stored at `~/.ycloud/config.toml`. The Dashboard defaults to `https://www.ycloud.com`. Override the config path with `YCLOUD_CONFIG` and the Dashboard base URL with `YCLOUD_DASHBOARD_URL`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.

## License

MIT
