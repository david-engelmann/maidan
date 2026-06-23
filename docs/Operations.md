# Operations

How to operate the repo day-to-day. The Architecture file says *what
the system is*; this file says *what you do to it*.

> Read [`CLAUDE.md`](../CLAUDE.md) first if you have not.

## Daily commands

```sh
# Full local CI before opening any PR
cargo fmt --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace            # requires Docker for integration tests

# Run the server against in-memory SQLite (no Docker)
DATABASE_URL=sqlite::memory: cargo run --bin maidan-server

# Run the prod-style stack (postgres + minio + server)
docker compose --profile full up
curl http://localhost:8080/health

# Two-instance federation push smoke (postgres + maidan-a + maidan-b)
docker compose --profile federation up -d
bash scripts/federation-smoke.sh

# Build the published docs site (mdBook)
cargo run -p maidan-mcp --bin gen-mcp-reference -- book/src/mcp-reference.md
mdbook build book
mdbook serve book   # preview at http://127.0.0.1:3000
```

## PR flow (the long version)

### 1. Pick the next item

The cluster's plan doc (`docs/Clusters/Cluster X.md`) lists PRs in
order with the linked Issue numbers. Work them in order unless you
have a reason to swap; PR `X.N+1` is usually written assuming
`X.N` shipped.

If you are starting a new cluster, write the plan doc first (see
"Cluster kickoff" below).

### 2. Branch + commit

```sh
git checkout main
git pull --ff-only
git checkout -b <kind>/<scope>-<slug>
```

- `kind ∈ {feat, chore, build, ci, docs, test, refactor}`
- `scope` is usually a crate name (`maidan-store`) or a concept
  (`workspace-scaffold`, `cluster-c-retro`).
- `slug` is short and lowercase with dashes.

Examples: `feat/maidan-search`, `ci/release-darwin-x86`,
`docs/cluster-c-retro`.

Commit with [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(maidan-search): pgvector embeddings + semantic search
chore: governance + workspace scaffold
docs(retro): Cluster C retrospective + v0.2.0 tag prep
ci: build x86_64-apple-darwin on macos-13
```

The PR title is the commit title is the squash-merge commit title.
Make it readable as a release-notes line.

### 3. Open the PR

```sh
git push -u origin <branch>
gh pr create --base main --head <branch> --title "..." --body "..."
```

The PR body **must** follow the template in
[`docs/Conventions.md`](Conventions.md). The Retrospective section
(per-PR) is mandatory — it survives squash-merge as part of the
commit body.

The template:

```markdown
## What this PR does
<2-4 bullets>

## Linked cluster
[Cluster X — Theme](docs/Clusters/Cluster%20X.md) · Phase X.N.

## Acceptance test
<the command(s) the reviewer runs to verify green>

## Risk / rollback
<what reverts cleanly; what doesn't>

## Out of scope
<things deferred to which PR>

## Retrospective (PR-level)
- **What was surprising:** <one or two; "nothing surprising" is acceptable>
- **What got deferred:** <bullets; each links to the future PR or follow-up issue>
- **What we learned:** <if any; otherwise omit>

Closes #<issue>.
```

### 4. Watch CI

```sh
gh pr checks <num>                 # one-shot
gh pr checks <num> --watch         # watch to completion
```

Or arm a `Monitor` and keep working — the harness will notify when
checks land.

The 8 required jobs:
- `lint (fmt + clippy + deny)` — ~30s
- `secrets scan` — ~10s
- `unit tests` — ~1m
- `integration (testcontainers)` — ~1m20s
- `docker compose smoke` — ~4m
- `scale-out smoke` — ~9m (required as of the `maidan-scale-1.0` gate, Cluster 120)
- `promtool (alert rules)` — ~10s (required as of Cluster 124)
- `otlp smoke` — ~9m (required as of Cluster 124)

If anything goes red, fix on the branch and push again. The most
common failures and fixes are in "Debugging CI" below.

### 5. Merge

```sh
gh pr merge <num> -R david-engelmann/maidan --squash --admin --delete-branch
```

The `--admin` flag is intentional. See
[`docs/Decisions.md`](Decisions.md) for the rationale.

After merge:

```sh
git checkout main
git pull --ff-only
git branch -d <branch>
```

## Cluster kickoff

When starting cluster `X`:

1. Create labels (one-time per cluster):

   ```sh
   gh label create cluster-x --color "0e8a16" --description "Cluster X work" --repo david-engelmann/maidan
   ```

2. Create the PR-tracker issues. Each PR in the cluster's plan has
   one issue, plus an `[X.retro]` issue:

   ```sh
   gh issue create --repo david-engelmann/maidan \
     --title "[X.1] Description" \
     --label cluster-x,<area-label> \
     --body "..."
   ```

3. Add issues to the Project board:

   ```sh
   for i in <issue-numbers>; do
     gh project item-add 1 --owner david-engelmann \
       --url "https://github.com/david-engelmann/maidan/issues/$i"
   done
   ```

4. Write `docs/Clusters/Cluster X.md` with the PR ladder, ordering
   rationale, exit criteria, and risks. Use Cluster A/B/C as
   templates.

5. Update `docs/Roadmap.md`'s "Current cluster" pointer.

6. Open PR `X.1` and start the loop.

## Cluster close

When PRs `X.1` through `X.N` are merged:

1. Open the `[X.retro]` PR on branch `docs/cluster-x-retro`.

2. Create `docs/Retros/Cluster X.md` per the shape in
   [`docs/Retros/README.md`](Retros/README.md). Every section is
   mandatory:
   - What shipped (one bullet per PR, with the merge commit SHA)
   - What was deferred (table: To, What, Why)
   - Surprises
   - Decisions (link to `docs/Decisions.md` if any locked
     differently)
   - Capability table extension
   - Risks identified + mitigated
   - Risks identified + still open
   - Forward look
   - Acknowledgements

3. Update:
   - `docs/Capabilities.md` — prepend the `v0.X.0` row
   - `CHANGELOG.md` — add `[0.X.0]` section with Added / Changed /
     Removed / Fixed / Security
   - `README.md` — refresh "What's in v0.X.0" + Status line
   - `docs/Architecture.md` — refresh the "at v0.X.0" header and any
     deferred-vs-shipped subsections
   - `docs/Roadmap.md` — mark cluster complete (✓), shift "Current
     cluster" pointer to next cluster
   - `docs/Retros/README.md` — add to the index

4. Merge the retro PR.

5. **Tag the release locally first:**

   ```sh
   git checkout main
   git pull --ff-only
   git tag -a v0.X.0 -m "Cluster X: <theme>.

   <one-paragraph summary of what's in this release>

   See CHANGELOG.md [0.X.0] and docs/Retros/Cluster X.md for the full retro."
   git tag -l v0.X.0 -n20  # verify the message
   ```

   Tag signing: no GPG signing key is configured, so tags are **annotated
   but unsigned** (the standing convention — see [`docs/Decisions.md`](Decisions.md)).
   To enable GPG-signed tags, set `git config user.signingkey <key>`, add the
   public key to GitHub, and use `-s` instead of `-a`. (Release *artifacts* are
   already signed keylessly via cosign — see step 7.)

6. **Push the tag** — this fires
   [`.github/workflows/release.yml`](../.github/workflows/release.yml):

   ```sh
   git push origin v0.X.0
   ```

   The workflow builds:
   - `x86_64-unknown-linux-gnu` on `ubuntu-latest`
   - `aarch64-unknown-linux-gnu` on `ubuntu-latest` via `cross`
   - `aarch64-apple-darwin` on `macos-latest`
   - `x86_64-apple-darwin` on `macos-13`

   Plus multi-arch ghcr.io images:
   - `ghcr.io/david-engelmann/maidan-server:v0.X.0`
   - `ghcr.io/david-engelmann/maidan-postgres:v0.X.0`

   Plus a GitHub Release with the binaries attached.

7. Verify the Release at `https://github.com/david-engelmann/maidan/releases/tag/v0.X.0`.
   If anything failed, see "Debugging the release workflow" below.
   The workflow attaches `sbom.json` (cyclonedx) and **keyless cosign
   (Sigstore) signatures** for every release artifact (Track V.3): each
   `*.tar.gz` and `sbom.json` ships with a self-verifiable `.cosign.bundle`,
   signed via the workflow's GitHub OIDC identity (no private key). Verify:

   ```sh
   cosign verify-blob --bundle maidan-<target>.tar.gz.cosign.bundle \
     --certificate-identity-regexp '^https://github.com/david-engelmann/maidan' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com \
     maidan-<target>.tar.gz
   ```

8. Open the next cluster kickoff.

## Debugging CI

### `lint` fails

- `cargo fmt --check` failed: run `cargo fmt` locally, commit, push.
- `clippy -D warnings` failed: read the lint, fix it. If a lint is
  wrong, use `#[allow(clippy::...)]` with a `// reason: ...` comment
  explaining why.
- `cargo deny check` failed:
  - `unmaintained` advisory: if it's a dev-dep with no production
    impact, add to `deny.toml`'s `[advisories] ignore` with a
    rationale comment.
  - `wildcard` error: workspace path deps need `publish.workspace =
    true` on the crate and `publish = false` in workspace.package.
  - `vulnerability`: check if a fixed version exists; bump deps or
    ignore with rationale if the vulnerability is not reachable in
    our code path.

### `secrets` fails

trufflehog found a verified secret. Treat as a real incident:

1. Rotate the secret immediately at the issuer.
2. Force-push a history rewrite to remove it (or contact GitHub
   support if it's already on `main`).
3. Investigate how it got committed; fix the discipline gap.

If trufflehog itself is broken (the action API changed): pin to a
specific commit SHA in `ci.yml`.

### `unit tests` fails

Run `cargo test --lib --bins --workspace` locally with the same
toolchain. The toolchain pin is in `rust-toolchain.toml`; if a deep
transitive dep needs a newer rustc, bump the pin.

### `integration (testcontainers)` fails

Run `cargo nextest run --workspace --tests` locally with Docker
running. Common failures:

- "syntax error at or near `(`": a migration uses syntax that the
  testcontainer's Postgres major doesn't support. Verify the test is
  pinned to `pgvector/pgvector:pg17` (not `postgres:17-alpine`); the
  pg17 image supports everything pg16 supports plus the `vector`
  extension.
- "cannot DELETE from contentless fts5 table": the FTS5 schema was
  reverted to `content=''`. It must stay non-contentless.
- "docker unavailable": expected on CI runners without DinD. The
  test's `match Postgres::default().start().await { Err(..) => return,
  ... }` pattern handles this; if it still fails, the pattern was
  removed.

### `coverage (llvm-cov)` fails

The CI coverage job now enforces a line-coverage floor with
`--fail-under-lines` in `.github/workflows/ci.yml`.

- Reproduce locally:

  ```sh
  COVERAGE_MIN_LINES=9.0 \
  cargo llvm-cov --workspace --lib --bins \
    --fail-under-lines "$COVERAGE_MIN_LINES"
  ```

- Baseline for the initial gate: **9.8%** line coverage from green main
  run `26485125992` (gate set slightly lower at `9.0` to avoid noise).
- **Cluster 5.0** raised the floor to **`10.0`** after targeted unit tests
  (filters, subscribe resume, listener health). Green run `26492169902` (11.0
  failed on first attempt). Re-measure on `main` before the next bump.
- **Cluster 9.0** raised the floor to **`10.5`** after targeted tests in
  `maidan-types` (`EventFilter`), `maidan-bus` (hydrate/error), `maidan-server`
  (subscribe metrics, hydrate `/metrics` e2e), `maidan-search`, and `maidan-auth`.
- **Cluster 11.0** raised the floor to **`11.0`** after outbox/relay coverage
  (PR #173; green CI run `26529705006`). Re-measure on `main` before the next bump.
- If the floor needs to move, do it in a dedicated CI/docs PR and note
  the run id used for recalibration.

### Codecov (optional)

When `CODECOV_TOKEN` is configured as a repository secret, the coverage job
uploads `lcov.info` via `codecov/codecov-action`. Fork PRs and local runs skip
the upload step. The upload does not fail CI when Codecov is unreachable.

### Subscribe delivery troubleshooting (`v6.0.0`)

1. Reproduce lag locally: `cargo test -p maidan-server subscribe_emits_replay_hint_when_bus_subscriber_lags -- --nocapture`.
2. Scrape metrics: `curl -s localhost:8080/metrics | rg 'maidan_(bus_lag|subscribe_replay)'`.
3. **No workspace filter** — subscribers without `filter.workspace_id` only get
   `replay_hint`, not auto-replay; see [[Production#Delivery reliability metrics]].
4. **Truncation loop** — sustained `replay_truncated` means the client must advance
   `after_id` until the frame stops; see [[Clusters/Cluster 4.0]].
5. **Postgres LISTEN** — `maidan_bus_listener_ok` and `/health/ready` `bus` field;
   listener errors increment `maidan_bus_listener_errors_total`.
6. **Indexer silence** — set `INDEXER_STALE_SECS` (e.g. `300`) when embeddings are on;
   watch `maidan_indexer_last_event_age_seconds` and `/health` `indexer_last_event_at`.

### Bus hydrate troubleshooting (`v8.0.0`)

1. Reproduce missing row: `cargo test -p maidan-bus pointer_notify_for_missing_log_id_increments_not_found_hydrate_stat -- --nocapture` (requires Docker).
2. Scrape metrics: `curl -s localhost:8080/metrics | rg 'maidan_bus_notify_hydrate'`.
3. **Spike in `not_found`** — confirm HTTP mutations call `append_event` before `bus.publish`; check for federation or scripts calling `pg_notify` directly.
4. **Spike in `invalid_payload`** — inspect NOTIFY payloads in logs (`drop notify payload`); legacy full-envelope path still requires valid JSON.
5. **Subscriber gaps with flat hydrate counters** — use subscribe replay metrics ([[Production#Delivery reliability metrics]]); hydrate failures are listener-side only.

### `docker compose smoke` fails

- "wait for /health timed out": the maidan-server container didn't
  start in 120s. Check the `compose logs on failure` step output for
  why — usually a migration failure or a connection refused on
  Postgres because of healthcheck race.

If a healthcheck race recurs, increase the healthcheck retries in
`compose.yaml` or extend the `for i in 1..60` loop in `ci.yml`.

## Debugging the release workflow

If the release workflow runs but doesn't produce a GitHub Release:

1. Check the per-matrix-job status:

   ```sh
   gh run list --repo david-engelmann/maidan --workflow=release.yml --limit 5
   gh run view <run-id> --repo david-engelmann/maidan --log-failed | tail -40
   ```

2. The **`bundle`** job downloads the three `maidan-*` matrix artifacts by
   name, flattens them into one `release-assets` artifact, and the
   **`github release`** job downloads only that bundle. **Docker push is
   separate** — a slow or failed image build no longer blocks GitHub
   Release assets.

3. Common failures:
   - **`download-artifact` fails after some artifacts succeed**: the release
     job was pulling every workflow artifact (including Docker GHA cache
     blobs). Fixed by bundling named `maidan-*` artifacts first.
   - **`maidan-server` docker exceeded 2h** (historical): sequential
     multi-arch in one job. The workflow now builds `linux/amd64` and
     `linux/arm64` in parallel, then merges with `docker buildx imagetools`.
   - **Workflow stuck hours on `macos-13`**: Intel Mac builds moved to
     [`.github/workflows/release-darwin-x86.yml`](../.github/workflows/release-darwin-x86.yml)
     (`workflow_dispatch` only). They are not part of the tag release path.
   - macOS x86_64 build red on `macos-latest`: the runner is arm64
     now. Use `release-darwin-x86.yml` on `macos-13`. See PR #36.
   - Docker push fails on auth: check that the runner has
     `packages: write` permission in `release.yml`.
   - `softprops/action-gh-release` fails on
     `fail_on_unmatched_files`: one or more matrix builds didn't
     produce an artifact. Fix the matrix entry that failed.

4. To retry a release without re-tagging:

   ```sh
   gh workflow run release.yml --repo david-engelmann/maidan \
     -f tag=v0.X.0
   ```

5. To create a release manually after the workflow already failed:

   ```sh
   gh release create v0.X.0 --repo david-engelmann/maidan \
     --title "v0.X.0 — Cluster X: <theme>" \
     --notes-file <(echo "...")
   ```

## Branch protection state

`main` is protected. As of v0.2.0:

- 8 required status checks: `lint (fmt + clippy + deny)`,
  `secrets scan`, `unit tests`, `integration (testcontainers)`,
  `docker compose smoke`, `scale-out smoke` (promoted to required
  at the `maidan-scale-1.0` gate, Cluster 120), and `promtool (alert
  rules)` + `otlp smoke` (promoted in Cluster 124).
- 1 required PR review (the maintainer self-merges via `--admin`
  bypass).
- No force push.
- No deletions.
- Required conversation resolution.
- Required linear history (squash-merge only).
- `strict = true` (PR must be up-to-date with `main` before merge).

To inspect:

```sh
gh api /repos/david-engelmann/maidan/branches/main/protection | jq
```

To update (rare):

```sh
gh api -X PUT /repos/david-engelmann/maidan/branches/main/protection \
  --input <branch-protection.json>
```

A template `branch-protection.json` is generated in this session's
shell history; otherwise reconstruct from the JSON in this section.

## Project board

[Maidan Roadmap](https://github.com/users/david-engelmann/projects/1)
is the GitHub Project v2 board. Every issue gets added at creation
time via:

```sh
gh project item-add 1 --owner david-engelmann \
  --url "https://github.com/david-engelmann/maidan/issues/<num>"
```

The board has the default `Backlog` / `Planned` / `In progress` /
`In review` / `Done` columns. Moving between columns is currently
manual; future automation is a Cluster X candidate.

## When the repo is in a half-state

If something breaks mid-cluster (e.g., the user interrupts a long
session):

1. Check `git status` and `git log --oneline -10`.
2. Read the most recent retro for context.
3. Read the most recent open PR's body for what was in flight.
4. Read [`docs/Open Work.md`](Open%20Work.md) for what's queued.
5. If a branch was left uncommitted, decide:
   - Squash into a new commit and finish the PR.
   - Reset the branch (`git reset --hard origin/<branch>`) if the
     work is unwanted.

Never force-push to `main`. Branch resets are fine.
