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
4. Bootstrap the five npm packages with a maintainer account and 2FA.
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

```bash
npm run check:release
cargo fmt -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
npm run test:npm
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds native binaries on GitHub-hosted macOS and Linux runners, creates a private-repository GitHub Release for maintainers, publishes the npm packages, and updates the public Homebrew tap from the native npm tarballs.
