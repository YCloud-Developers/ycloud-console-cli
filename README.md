# ycloud - YCloud Console CLI

`ycloud` is the Console/Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `ycloud login` uses Dashboard browser grant + PKCE, receives the code through a localhost callback by default, and stores `YCLI.` tokens.
- `ycloud whoami` reads the current Console CLI identity.
- `ycloud tenants list` lists tenants available to the current Console CLI token.
- `ycloud integrations status` and `ycloud contacts metadata` use the stable `/api/cli/v1/**` contract.
- `ycloud inbox conversations export` creates one permission-scoped asynchronous task containing
  conversations, ordered message JSONL parts, a sanitized manifest, and a separate contacts
  artifact by default.
- `ycloud contacts export` provides the same artifact workflow for contact filters without first exporting conversations.
- `ycloud contacts stats` counts newly created contacts for an RFC 3339 time range and can filter or group by Contact Source without paging through contact details.
- `ycloud campaign list`, `messages`, and `analytics` query up to 100 tenant-scoped Campaigns; message rows are metadata-only and use an opaque keyset cursor.
- `ycloud campaign export` streams up to 1,000,000 metadata-only message rows as independently verified JSONL parts plus a manifest.
- `ycloud whatsapp waba-assignment list` automatically pages and joins the current WABA phone inventory, direct and team-expanded human assignments, and normalized routing rules. It never treats a partial backend failure as an unassigned phone.
- `ycloud analytics overview`, `trend`, and `templates` use the stable WhatsApp analytics contract. The CLI sends RFC 3339 time values and an IANA timezone such as `Asia/Shanghai`.
- Business commands use only the stable `/api/cli/v1/**` contract and never fall back to `/api/cli/read/**`. The legacy adapters remain server-side for CLI v0.1.3 compatibility.
- Every HTTP attempt sends a new `X-Request-Id`; attempts from one command share a non-secret invocation ID and mode.
- `ycloud contacts list`, `ycloud analytics logs`, and `ycloud analytics calling-logs` use stable web-owned v1 contracts. `YCLI.` tokens do not call ordinary Dashboard paths.
- `ycloud refresh` rotates the refresh token.
- `ycloud logout` revokes the current token and removes the local profile.

Manual copy-paste authorization code input is still available with `ycloud login --manual` for terminals that cannot receive a localhost browser callback.

## Standard OAuth trial

The development build supports the unified OAuth server without changing existing profiles:

```bash
ycloud --dashboard-url https://www-test-indigo.ycloud.com --config /tmp/ycloud-oauth-test.toml login --oauth --profile basic
ycloud --config /tmp/ycloud-oauth-test.toml whoami
ycloud --config /tmp/ycloud-oauth-test.toml refresh
ycloud --config /tmp/ycloud-oauth-test.toml logout
```

`--oauth` discovers the authorization server and `/api/cli` resource metadata, uses the
pre-registered public client `ycloud-console-cli`, and validates PKCE, state and callback
issuer. The localhost callback is `http://127.0.0.1:<random-port>/oauth/callback`.
`--no-browser` prints the URL while retaining the callback listener. Standard OAuth does
not support manual code pasting or scripted `--code`/`--state`/`--code-verifier` overrides.

The test ingress must forward `/.well-known/oauth-authorization-server` and
`/.well-known/oauth-protected-resource/api/cli`, plus `/api/oauth/v1/`, to `ycloud-open-platform:8080`. MCP resource discovery is
independent and is not required for this CLI trial. HTTPS is required except for
127.0.0.1 mock servers with the explicit `YCLOUD_OAUTH_TEST_LOOPBACK=1` switch. Default login remains the legacy protocol until the standard
server is available in every supported environment; standard failures never fall back.

OAuth options `--oauth-issuer`, `--oauth-client-id`, and `--oauth-resource` separate the trusted authorization server from the business origin supplied with `--dashboard-url`. Defaults retain the current same-origin CLI resource.

Both metadata documents are fetched from the explicit trusted issuer. With separate domains, set `--dashboard-url` to the existing Web origin, `--oauth-issuer` to the OAuth service origin, and `--oauth-resource` to `<web-origin>/api/cli`. The Web domain needs no new discovery route.

Saved version 2 standard sessions bind issuer, client, resource, business origin and discovered token/revoke endpoints. Endpoints must use the issuer origin; business requests must remain under `/api/cli/`. Redirects and environment proxies are disabled for standard OAuth and its business calls. Old trial sessions upgrade only after same-origin trusted discovery, retaining the original grant. Unsupported or inconsistent versions require a new login.

Saved standard sessions bind issuer, client and resource. Business commands reject a
foreign `--dashboard-url`, and refresh/logout select the saved protocol. Refresh atomically
replaces the token pair and rejects scope expansion; code exchange and rotation are not
retried. Before rotation, the CLI atomically marks its result as unconfirmed; an interruption requires a new login instead of reusing the old refresh token. Logout removes local credentials even if the server cannot confirm revocation, reports that distinction, and directs the user to YCloud authorization management. Existing legacy profiles continue
to use their old refresh/revoke endpoints. No npm/Homebrew release is part of this trial.

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

Release packages support macOS, Linux, and Windows on ARM64 and x64.
Windows users can install the same npm launcher from PowerShell, Command Prompt, or a
Node.js-enabled terminal:

```powershell
npm install --global @ycloud-ai/console-cli
ycloud --version
```

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
ycloud contacts stats --start-time 2026-06-28T00:00:00+08:00 --end-time 2026-07-28T00:00:00+08:00 --group-by source
ycloud inbox conversations search --inbox-id inbox-1 --status all --limit 100
ycloud inbox conversations export --inbox-id inbox-1 --status all --output-dir ./export
ycloud contacts export --condition vip --format csv --output-dir ./export
ycloud campaign list --waba-id WABA_ID --page-size 20
ycloud campaign messages --campaign-id CAMPAIGN_1,CAMPAIGN_2 --start-time 1782921600000 --end-time 1783526400000 --timezone Asia/Shanghai
ycloud campaign analytics --campaign-id CAMPAIGN_1 --campaign-id CAMPAIGN_2 --start-time 1782921600000 --end-time 1783526400000 --dimension campaign,date
ycloud campaign export --campaign-id CAMPAIGN_1,CAMPAIGN_2 --start-time 1782921600000 --end-time 1783526400000 --output-dir ./export
ycloud whatsapp waba-assignment list --waba-id WABA_ID
ycloud whatsapp waba-assignment list --json
ycloud exports query TASK_ID
ycloud exports download TASK_ID --artifact conversations,messages,contacts,manifest --output-dir ./export
ycloud exports download TASK_ID --artifact campaign-messages,manifest --output-dir ./export
ycloud analytics outline
ycloud analytics overview
ycloud analytics trend --preset last-30-days
ycloud analytics templates list --preset last-90-days
ycloud analytics templates export --preset last-90-days --output-dir ./export
ycloud analytics logs --page-no 1 --page-size 20
ycloud analytics calling-logs --page-no 1 --page-size 20
ycloud refresh
ycloud logout
```

Inbox conversation search and export default to `--status all`. Use `--status open` or
`--status closed` to select one state; the legacy aliases `1` (open) and `0` (closed) remain
accepted.

Export creation waits by default, then downloads the independent ready artifacts. Use
`--no-wait` to return the task ID immediately. `--archive` requests an additional ZIP when the
server-side feature is enabled; ZIP never replaces the conversation, contact, or manifest files.
Downloads are streamed through a `.part` file, verified against the server-provided size and
SHA-256 checksum, and never overwrite an existing file.

Campaign message and analytics ranges are half-open `[startTime,endTime)` and may span at most 90
natural days in the supplied IANA timezone. Campaign IDs may be repeated or comma-separated, but
must be unique and are capped at 100. Campaign export has no feature switch: it is controlled by
the explicit `yc.campaign.message.export` permission, hard server limits, rate limits, and an
isolated background runtime budget. When Campaign JSONL parts are downloaded, the CLI also
downloads the manifest, verifies every part's size and SHA-256, and rejects missing, duplicate, or
non-contiguous parts. Task cancellation is intentionally not exposed by this CLI capability.

Analytics commands default to the previous 7 completed natural days in the requested IANA timezone and accept at most 90 natural days. Presets end at the start of the current local day. Ranges up to 30 days use daily trend buckets; larger ranges use weekly buckets. Delivery rate is calculated from the aggregated delivered and total counts, and an empty denominator is printed as `-` in tables and `null` in JSON.

Template statistics use the Dashboard's fixed page size of 10. Message-type rows contain total, delivered, and weighted delivery rate; failed and processing counts are available only for each overall trend bucket.

CLI flags accept millisecond timestamps and convert them to RFC 3339 before sending. Analytics defaults to the IANA timezone `Asia/Shanghai`. Filters are repeatable; comma-separated `--message-category` is not part of the 0.2.0 interface. Table output is the default and `--json` selects machine-readable output:

```bash
ycloud analytics overview --start-time 1782921600000 --end-time 1783526400000 --from 8613800138000 --region-code CN --message-type marketing --message-type utility --waba-id WABA_ID
ycloud analytics trend --preset last-60-days --json
ycloud analytics templates list --preset last-90-days --template-name welcome --sort-field delivery-rate
ycloud analytics templates export --preset last-90-days --file-name template-statistics --no-wait
ycloud analytics logs --start-time 1782921600000 --end-time 1783526400000 --direction OutBound --status sent,delivered --source "WhatsApp Business API"
ycloud analytics calling-logs --start-time 1782921600000 --end-time 1783526400000 --directions BUSINESS_INITIATED --sources CALLING --status COMPLETED
```

Analytics is operational reporting and must not be used for billing reconciliation. `overview` requests the summary, trend, and first template page concurrently. It prints successful sections even if another section fails, then exits nonzero. Large template result sets use the existing asynchronous export lifecycle and produce `TEMPLATE_STATISTICS` plus manifest artifacts.

Contact statistics accepts a maximum 90-day half-open range `[startTime,endTime)`. Repeat `--source` to filter several sources; combine it with `--group-by source` to group only the selected sources. The command always prints stable JSON. Campaign commands also print stable JSON so cursors, availability, and task metadata remain machine-readable.

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
requested permissions. Inbox conversation, Inbox message, and Contact export permissions are
active but require explicit
enablement and are not included in any default profile:

```bash
ycloud login --profile readonly \
  --permission yc.inbox.conversation.export \
  --permission yc.inbox.message.export \
  --permission yc.contact.record.export
```

The WABA assignment scenario also uses explicit permissions. Its human-agent assignment
permission is HIGH risk because the response contains agent email:

```bash
ycloud login --profile basic \
  --permission yc.whatsapp.phone.read \
  --permission yc.inbox.phone-assignment.read \
  --permission yc.inbox.assignment-rule.read
```

Campaign search is included in `readonly` and `analytics-read`, but Campaign message export is an
explicit HIGH-risk permission:

```bash
ycloud login --profile analytics-read \
  --permission yc.campaign.message.export
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

Config is stored at `~/.ycloud/config.toml` (`%USERPROFILE%\.ycloud\config.toml` on Windows). The Dashboard defaults to `https://www.ycloud.com`. Override the config path with `YCLOUD_CONFIG` and the Dashboard base URL with `YCLOUD_DASHBOARD_URL`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.

## License

MIT
