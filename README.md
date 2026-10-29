# ycloud - YCloud Console CLI

`ycloud` is the Console/Dashboard-oriented YCloud CLI. It is intentionally separate from the existing `ycli` OpenAPI/API-key CLI.

## Scope

- `ycloud login` always uses standard OAuth discovery and PKCE S256, opens the browser consent flow, and receives the code through a validated localhost callback.
- `ycloud whoami` reads the current Console CLI identity.
- `ycloud tenants list` lists tenants available to the current Console CLI token.
  Delegated employee sessions expose only their fixed target tenant and cannot switch workspaces.
- `ycloud integrations status` and `ycloud contacts metadata` use the stable `/api/cli/v1/**` contract.
- `ycloud inbox conversations export` creates one permission-scoped asynchronous task containing
  conversations, ordered message JSONL parts, a sanitized manifest, and a separate contacts
  artifact by default.
  Message task queries and downloads retain `messageCompleteness` (`COMPLETE`, `PARTIAL`, or
  `UNVERIFIED`); historical message tasks without verified accounting are not reported as complete.
- `ycloud contacts export` provides the same artifact workflow for contact filters without first exporting conversations.
- `ycloud contacts stats` counts newly created contacts for an RFC 3339 time range and can filter or group by Contact Source without paging through contact details.
- `ycloud campaign list`, `messages`, and `analytics` query up to 100 tenant-scoped Campaigns; message rows are metadata-only and use an opaque keyset cursor.
- `ycloud campaign export` streams up to 1,000,000 metadata-only message rows as independently verified JSONL parts plus a manifest.
- `ycloud whatsapp waba-assignment list` uses the platform-owned WABA report to retrieve the complete phone inventory, direct and team-expanded human assignments, and normalized routing rules. It never treats a partial backend failure as an unassigned phone.
- `ycloud analytics overview`, `trend`, and `templates` use the stable WhatsApp analytics contract. The CLI sends RFC 3339 time values and an IANA timezone such as `Asia/Shanghai`.
- Business commands use only the stable `/api/cli/v1/**` contract and never fall back to `/api/cli/read/**`. Old Web business routes return HTTP410 with upgrade guidance.
- Every HTTP attempt sends a new `X-Request-Id`; attempts from one command share a non-secret invocation ID and mode.
- `ycloud contacts list`, `ycloud analytics logs`, and `ycloud analytics calling-logs` use stable web-owned v1 contracts. CLI tokens do not call ordinary Dashboard paths.
- `ycloud refresh` rotates the refresh token.
- `ycloud logout` revokes the current token and removes the local profile.

Use `ycloud login --no-browser` to open the printed authorization URL yourself while keeping the localhost callback listener running. Legacy manual-code flags are no longer accepted.

## Standard OAuth

`ycloud login` uses the unified OAuth server by default:

```bash
ycloud login
ycloud whoami
ycloud refresh
ycloud logout
```

Production defaults to issuer and business origin `https://open-platform.ycloud.com`.
During explicit login, the CLI takes the logical `/api/cli` resource from that trusted issuer's
protected-resource metadata. The logical audience may remain `https://www.ycloud.com/api/cli`;
it is not the HTTP destination. Custom business addresses still require an explicit issuer.
When the business and issuer origins differ, the resource must remain on the business origin.
For a platform environment, use its platform origin for both:

```bash
ycloud --dashboard-url https://open-platform-prepare.ycloud.com \
  --config /tmp/ycloud-prepare.toml login --profile basic \
  --oauth-issuer https://open-platform-prepare.ycloud.com
```

Login discovers the authorization server and `/api/cli` resource metadata, uses the
pre-registered public client `ycloud-console-cli`, and validates PKCE, state and callback
issuer. The localhost callback is `http://127.0.0.1:<random-port>/oauth/callback`.
`--oauth` remains an accepted, hidden no-op for existing scripts. There is no legacy login
mode or protocol fallback. `--manual`, `--code`, `--state` and `--code-verifier` are rejected.

The local callback displays a self-contained YCloud result page in English or Chinese,
with light/dark appearance and an explicit manual-close hint. Browser authorization
completion is separate from token exchange: wait for the CLI's login success message in
the originating terminal. The callback page does not infer a desktop application or launch
an arbitrary URI. Close the browser tab manually after checking the terminal. The CLI must
remain running while authorization is in progress; its callback wait expires after 300 seconds.

The OAuth service ingress must forward `/.well-known/oauth-authorization-server` and
`/.well-known/oauth-protected-resource/api/cli`, plus `/api/oauth/v1/`, to `ycloud-open-platform:8080`. MCP resource discovery is
independent and is not required for CLI authorization. HTTPS is required except for
127.0.0.1 mock servers with the explicit `YCLOUD_OAUTH_TEST_LOOPBACK=1` switch. Discovery or authorization failure never falls back to legacy authentication.

OAuth options `--oauth-issuer`, `--oauth-client-id`, and `--oauth-resource` separate the trusted authorization server from the business origin supplied with `--dashboard-url`. The issuer metadata remains authoritative; an explicit --oauth-resource must exactly match it.

Both metadata documents are fetched from the explicit trusted issuer. New version3 sessions bind
issuer, client, advertised resource, business origin and discovered token/revoke endpoints. Only an
explicit login adopts a logical audience on a different origin when business and issuer are the
same platform origin. Existing version1/2 profiles retain their original stricter bindings and are
never rewritten by read-only commands. All token requests remain form encoded with explicit
resource; the WorkBuddy-only JSON or missing-resource compatibility does not apply to CLI.

The CLI does not create or open profile lock files. Read-only queries, including `whoami`, use an atomic profile snapshot without updating the profile. They work with a readable config in a read-only directory, including original same-origin standard OAuth trial profiles. Login, explicit `refresh`, logout and trial-profile upgrades still require write access; queries do not refresh tokens automatically. Profile saves remain atomic, but credential operations are not serialized across processes: concurrent refreshes can be rejected by server replay protection and revoke the grant, requiring a new login. Do not run concurrent credential updates against the same config. A failed or unknown refresh still requires a new login instead of retrying the old refresh token. Permission-recovery commands preserve the saved Web origin, issuer, client and resource. Business clients bind their address and credentials to the same snapshot. A replaced profile cannot send its credentials through an older client; retry the command after a profile change. Source builds require Rust 1.89 or later.

Saved standard sessions bind issuer, client and resource. Business commands reject a
foreign `--dashboard-url`, and refresh/logout use the saved standard endpoints. Refresh atomically
replaces the token pair and rejects scope expansion; code exchange and rotation are not
retried. Before rotation, the CLI atomically marks its result as unconfirmed; an interruption requires a new login instead of reusing the old refresh token. Logout removes local credentials even if the server cannot confirm revocation, reports that distinction, and directs the user to YCloud authorization management. Profiles created with legacy authentication require `ycloud login` again before queries, refresh or logout; their tokens are not converted or sent to either protocol. A successful new login replaces the local profile. Existing legacy grants can be revoked in YCloud authorization management. Old Web registered business routes return410 without redirect/retry; retired legacy login endpoints are not a login fallback. The CLI is distributed through the existing npm and Homebrew channels.

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

## Local OAuth Test

Run a mock or local OAuth service with matching resource metadata, and explicitly enable
loopback HTTP for testing:

```bash
YCLOUD_OAUTH_TEST_LOOPBACK=1 cargo run -- login \
  --dashboard-url http://127.0.0.1:8036 --oauth-issuer http://127.0.0.1:8036 \
  --profile readonly --no-browser
```

Open the printed URL in a browser. Keep the CLI running while the browser redirects to
`http://127.0.0.1:<random-port>/oauth/callback`; the callback must include the exact state
and issuer. Public deployments require HTTPS.

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

Analytics is operational reporting and must not be used for billing reconciliation. `overview` calls the shared platform composite once; the platform returns explicit summary, trend and first-template-page section outcomes. It prints successful sections even if another section fails, then exits nonzero. Large template result sets use the existing asynchronous export lifecycle and produce `TEMPLATE_STATISTICS` plus manifest artifacts.

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

Config is stored at `~/.ycloud/config.toml` (`%USERPROFILE%\.ycloud\config.toml` on Windows). The business origin defaults to `https://open-platform.ycloud.com`. Override the config path with `YCLOUD_CONFIG` and the Dashboard base URL with `YCLOUD_DASHBOARD_URL`.

Tokens are persisted in this first P0 implementation so the backend flow can be tested end to end. The production hardening path is system keychain first, file fallback with strict permissions and an explicit warning.

## License

MIT


## 0.5.0 platform cutover

Upgrade the binary, then explicitly run `ycloud login` to replace the saved Web-origin profile.
Existing TOML overrides and tokens are not moved automatically. The old Web Ingress is unchanged;
registered old CLI business methods return `cli_upgrade_required` (410), without automatic retry,
redirect or reauthorization. A legacy profile may fail local validation before HTTP; upgrade and
log in explicitly in that case. Old export task IDs cannot be queried, downloaded or retried on
the platform. Submit the original parameters as a new export. The unrelated API-key CLI and
`~/.ycli` remain unchanged. Historical standard OAuth refresh/revoke remain supported.
