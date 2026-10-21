# Release Guide

## Public Names

- GitHub source: `YCloud-Developers/ycloud-console-cli` (private)
- npm launcher: `@ycloud-ai/console-cli`
- executable: `ycloud`
- Homebrew tap: `YCloud-Developers/homebrew-tap`
- Homebrew formula: `ycloud`

## Prerequisites

1. Keep the source repository private and use the MIT license for distributed source and binaries.
2. Create `YCloud-Developers/ycloud-console-cli` as private and `YCloud-Developers/homebrew-tap` as public.
3. Create a protected GitHub environment named `release` with required reviewers.
4. Bootstrap the seven npm packages with a maintainer account and 2FA.
5. Configure every npm package to trust:
   - organization: `YCloud-Developers`
   - repository: `ycloud-console-cli`
   - workflow: `release.yml`
   - environment: `release`
6. After OIDC publishing succeeds, remove the bootstrap `NPM_TOKEN` secret and disallow token publishing on npm.
7. Add a write-enabled deploy key to `YCloud-Developers/homebrew-tap` and store its private key as the source repository secret `HOMEBREW_TAP_DEPLOY_KEY`.

The npm packages are public, but npm provenance attestations are unavailable because the publishing source repository is private. Homebrew downloads the native public npm tarballs and does not depend on private GitHub Release access.

## Release

Keep all versions equal in `Cargo.toml`, the root `package.json`, and every package under `npm/`.

### 0.3.1 local credential access

Version `0.3.1` removes local profile file locking. Release notes must state that:

- Queries with an existing usable token, including `whoami`, never write the profile or open a lock file. Legacy and standard profiles work in read-only config directories.
- Login, explicit refresh, logout and trial-profile migration no longer create or open sidecar lock files; credential saves remain atomic.
- Standard OAuth persists its unconfirmed-refresh marker before a one-time request and never automatically retries an unknown result. Writable config storage is still required for credential updates.
- Credential operations are no longer serialized across processes. Concurrent refreshes against the same grant can trigger server replay protection and require login again; successful concurrent refresh is not guaranteed.
- Default legacy login and opt-in standard OAuth endpoints remain unchanged. This release adds no server-side replay grace or idempotency window.

### 0.3.0 standard OAuth

Version `0.3.0` adds opt-in `login --oauth` with PKCE S256 and trusted issuer/resource discovery. Release notes must state that:

- Existing `ycloud login` and saved legacy profiles continue to use the compatible Web authorization, refresh and revoke interfaces.
- Production standard OAuth uses `--dashboard-url https://www.ycloud.com`, `--oauth-issuer https://open-platform.ycloud.com`, and `--oauth-resource https://www.ycloud.com/api/cli`.
- Standard OAuth does not fall back to the legacy protocol when authorization fails.
- Saved issuer, client, resource and business-origin bindings, serialized refresh, revoke, and the browser result page are included.

### 0.2.0 analytics cutover

Version `0.2.0` replaces the previous analytics overview interface. Release notes must call out that:

- `analytics overview`, `analytics trend`, and `analytics templates list|export` use the stable CLI v1 analytics contract.
- Analytics ranges are limited to 90 natural days; ranges up to 30 days use daily buckets and longer ranges use weekly buckets.
- `--message-type` is repeatable. The old comma-separated `--message-category` form is removed.
- Table output is the default; `--json` is the machine-readable option. There is no `--output table|json` compatibility flag.
- Template statistics exports use the existing asynchronous export task, wait, and download lifecycle.

```bash
npm run check:release
cargo fmt -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
npm run test:npm
git tag v0.3.1
git push origin v0.3.1
```

The workflow builds native binaries on GitHub-hosted macOS, Linux, and Windows runners, creates a private-repository GitHub Release for maintainers, publishes the npm packages, and updates the public Homebrew tap from the native npm tarballs. Windows ARM64 and x64 are distributed both as release ZIPs and through the npm launcher.

The publish job is safe to retry for the same tag. It skips npm package versions that already exist, reuses an existing draft GitHub Release, leaves an already-published release unchanged, and avoids an empty Homebrew commit when the formula is already current.
