# Cluster 298.0 retro — SDK release workflow

> Tag **`v298.0.0`**. Phase XXIV (post-gate hardening). Post-SDK-arc: publishing. No new gate tag.

## What shipped

The machinery to publish the four SDKs (294–297) to their registries, independently of the
server:

- **`.github/workflows/sdk-release.yml`** — publishes on per-language tags:
  `sdk-ts-vX.Y.Z` → npm, `sdk-py-vX.Y.Z` → PyPI, `sdk-rs-vX.Y.Z` → crates.io, and
  `sdk-go-vX.Y.Z` → re-tag the commit as **`sdk/go/vX.Y.Z`** (the module-path version Go
  tooling consumes — Go has no registry). Each job runs only for its own tag prefix and
  **fails fast if the tag version ≠ the package manifest version**. Registry auth via repo
  secrets `NPM_TOKEN` / `PYPI_TOKEN` / `CRATES_TOKEN`.
- **`docs/SDK Release.md`** — the tag→registry table, the secrets + how to set them, the
  cut-a-release steps, and the local dry-run commands (repo-only contributor doc).
- **`sdk/python/.gitignore`** (dist/build/egg-info/pyc) + **`release_secrets.txt` added to the
  root `.gitignore`** (a plaintext-token file must never be committed). Polished the npm
  `repository.url` (the `--dry-run` flagged it).

**Registry secrets loaded.** The maintainer provided the three tokens; all three are set as
GitHub Actions repo secrets (via `gh secret set` from stdin — values never printed or
committed). Verified all four packages publish-ready by **local dry-run**: `npm publish
--dry-run` (5 files, `maidan@0.1.0`), `cargo publish --dry-run` (packages + compiles),
`python -m build` + `twine check` (both artifacts PASSED), `go vet`/`go build`.

## Surprises / decisions

- **Per-language tags, not one unified `sdk-v*`.** Independent SemVer per client (the contract's
  intent) — release TypeScript without forcing a Rust bump. Each job's `if` gates on the tag
  prefix (handles both tag-push and `workflow_dispatch`).
- **Go "publishing" = a git tag.** Go modules are consumed by version tag at the module path
  (`github.com/david-engelmann/maidan/sdk/go`), so the version tag must be `sdk/go/vX.Y.Z`. The
  `sdk-go-v*` job re-tags the same commit as `sdk/go/vX.Y.Z` (default `GITHUB_TOKEN` +
  `contents: write`; a token-pushed tag doesn't re-trigger workflows, so no loop).
- **Version guard.** Each job compares the tag's version to the manifest and fails on mismatch —
  a tag can't silently publish a stale manifest version. Bump the manifest, merge, then tag.
- **Secrets handling.** `release_secrets.txt` was in the repo root (untracked, not ignored — a
  `git add -A` risk). Gitignored it first thing; loaded the tokens into GH secrets via stdin
  pipe (no value ever in a command line, log, or commit); recommended the maintainer delete the
  local file now that the tokens live in GH secrets.

## Capability table extension

New `sdk-release` CI workflow + `docs/SDK Release.md`. No server capability change; no SDK code
change (metadata polish only).

## Risks identified + still open

- **The packages are not yet published** — this cluster is the machinery + verified dry-runs.
  The real publish happens by pushing `sdk-{ts,py,rs,go}-v0.1.0` (reported separately). Name
  availability on npm/PyPI/crates.io is confirmed only at publish time (a name owned by someone
  else 403s).
- **`--provenance` (npm) / trusted publishing (PyPI OIDC)** deferred — token auth for 0.1.0;
  OIDC/provenance is a hardening follow-up.

## Forward look

Push the `sdk-*-v0.1.0` tags to publish, then **299** adds the report-only SDK interop CI job
(run the black-box suites across the four in CI). Then the rest of the five-arc program
(MCP `2026-07-28`, durable mail retry queue, Slack/Git projectors, launch).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 297.0]] and
the SDK arc ([[maidan-sdk-arc]]). Tokens provided by the maintainer under the standing publish
authorization; loaded to GH secrets, never committed.
