# ycloud - YCloud Console CLI

`ycloud` is the Console/Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `ycloud login` uses Dashboard browser grant + PKCE, receives the code through a localhost callback by default, and stores `YCLI.` tokens.
- `ycloud whoami` reads the current Console CLI identity.
- `ycloud tenants list` lists tenants available to the current Console CLI token.
- `ycloud integrations status` and `ycloud contacts metadata` use the stable `/api/cli/v1/**` contract.
- `ycloud inbox conversations export` creates a permission-scoped asynchronous export and includes a separate contacts artifact by default.
- `ycloud contacts export` provides the same artifact workflow for contact filters without first exporting conversations.
- `ycloud analytics outline` and `ycloud analytics overview` use the stable WhatsApp analytics contract. The CLI sends RFC 3339 time values and an IANA timezone such as `Asia/Shanghai`.
- Business commands use only the stable `/api/cli/v1/**` contract and never fall back to `/api/cli/read/**`. The legacy adapters remain server-side for CLI v0.1.3 compatibility.
- Every HTTP attempt sends a new `X-Request-Id`; attempts from one command share a non-secret invocation ID and mode.
- `ycloud contacts list`, `ycloud analytics logs`, and `ycloud analytics calling-logs` use stable web-owned v1 contracts. `YCLI.` tokens do not call ordinary Dashboard paths.
- `ycloud refresh` rotates the refresh token.
- `ycloud logout` revokes the current token and removes the local profile.

Manual copy-paste authorization code input is still available with `ycloud login --manual` for terminals that cannot receive a localhost browser callback.

## Installation

Install the CLI with npm (Node.js 18 or later):

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
ycloud inbox conversations search --inbox-id inbox-1 --limit 100
ycloud inbox conversations export --inbox-id inbox-1 --output-dir ./export
ycloud contacts export --condition vip --format csv --output-dir ./export
ycloud exports query TASK_ID
ycloud exports download TASK_ID --artifact conversations,contacts,manifest --output-dir ./export
ycloud analytics outline
ycloud analytics overview
ycloud analytics logs --page-no 1 --page-size 20
ycloud analytics calling-logs --page-no 1 --page-size 20
ycloud refresh
ycloud logout
```

Export creation waits by default, then downloads the independent ready artifacts. Use
`--no-wait` to return the task ID immediately. `--archive` requests an additional ZIP when the
server-side feature is enabled; ZIP never replaces the conversation, contact, or manifest files.
Downloads are streamed through a `.part` file, verified against the server-provided size and
SHA-256 checksum, and never overwrite an existing file.

Analytics commands default to the last 7 days. CLI flags continue to accept millisecond timestamps; stable WhatsApp analytics requests convert them to RFC 3339 before sending. `analytics overview` defaults to the IANA timezone `Asia/Shanghai`:

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

Profiles are expanded by the backend. The token and local config store only the resulting atomic
requested permissions. Inbox and Contact export permissions are active but require explicit
enablement and are not included in any default profile:

```bash
ycloud login --profile readonly \
  --permission yc.inbox.conversation.export \
  --permission yc.contact.record.export
```

When a business command receives the typed v1 response `HTTP 403` with
`error.code=permission_denied`, an interactive terminal shows the missing permission set and asks
before opening browser authorization. Existing custom permissions are retained, a valid new token
is saved only after authorization succeeds, and the original command is retried once. Automation
and non-TTY invocations never prompt or open a browser; they print the complete `ycloud login
--profile custom ...` remediation command and exit nonzero. If a permission was requested but is
not effective, update the account role or data range instead of requesting the same permission
again.

Export creation and retry are never automatically retried. Task polling and signed-artifact URL
requests are safe reads with bounded retries. Ordinary API requests have a 30-second total timeout;
artifact downloads are streamed independently so large files are not buffered in memory.
Create and retry commands generate an idempotency key by default. If a response is lost after
submission, repeat the command with the same `--idempotency-key` to recover the original task
without creating another export.

## Rate-limit retries

The CLI automatically retries only operations classified as `SAFE_READ`, and only when the
server returns HTTP 429 with `error.code=rate_limited` and `error.retryable=true`.
`Retry-After` takes precedence; otherwise the CLI uses exponential backoff with full jitter.
Authentication lifecycle requests and future writes are never retried.

Interactive mode allows at most 3 attempts, 5 seconds of cumulative waiting, and a
30-second overall deadline. Automation mode allows at most 4 attempts, 20 seconds of waiting,
and a 60-second deadline. Use `--automation`, or set
`YCLOUD_INVOCATION_MODE=automation`; without either, non-TTY execution selects automation
mode and terminal execution selects interactive mode. If the server's requested delay does
not fit the remaining budget, the command exits immediately and reports only safe
request/trace identifiers and retry guidance.

In the default browser flow, an authorization rejection returns to the localhost callback with `error`, `error_description`, and `state`. The CLI validates `state` before reporting the rejection, exits immediately, and does not exchange a token or write the profile.

## Config

Config is stored at `~/.ycloud/config.toml`. The Dashboard defaults to `https://www.ycloud.com`. Override the config path with `YCLOUD_CONFIG` and the Dashboard base URL with `YCLOUD_DASHBOARD_URL`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.

## License

MIT
