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
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 DATABASE_URL=sqlite::memory: cargo run --bin maidan-server

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

## Kill switches (operator levers)

When something is going wrong — a runaway agent, a leaked token, a traffic spike,
an untrusted egress target — these are the levers to pull. Two shapes: a
**per-member freeze** (a runtime API, no restart) and **`MAIDAN_*` env flags**
(set-and-restart).

### Freeze a member

Freezing a member drops their active leases (their claimed threads return to the
queue) and makes `claim_next` refuse them; they stay frozen until an explicit
unfreeze. It is **not** a thread/workspace pause — it stops one member. Requires
`token:admin`.

```bash
# freeze (optionally with an audit reason); returns the freeze + how many claims were released
curl -sX POST "$BASE/members/$MEMBER_ID/freeze" -H "Authorization: Bearer $ADMIN" \
     -H 'content-type: application/json' -d '{"reason":"compromised token"}'
curl -s "$BASE/workspaces/$WS/frozen-members" -H "Authorization: Bearer $ADMIN"   # who is frozen
curl -sX DELETE "$BASE/members/$MEMBER_ID/freeze" -H "Authorization: Bearer $ADMIN"  # unfreeze
```

MCP twins: `freeze_member` / `unfreeze_member` / `list_frozen_members` (also
`token:admin`). Freeze/unfreeze are written to the audit log
(`member.freeze` / `member.unfreeze`).

### `MAIDAN_*` env flags (set + restart)

| Flag | Lever |
|------|-------|
| `MAIDAN_ALLOW_INSECURE_NO_AUTH` (+ `AUTH_DISABLED`) | Auth is fail-closed: disabling it needs this explicit ack, and never in production. Leave unset in prod. |
| `MAIDAN_RATE_LIMIT_MAX` | Per-client request ceiling (per bearer/IP over 60 s). Unset ⇒ a built-in 1200/60 s floor on the server binary; `0` disables. Lower it to throttle a spike. |
| `MAIDAN_MAX_BODY_BYTES` | Max request body (default 2 MiB); oversized ⇒ `413`. |
| `MAIDAN_SECRET_EGRESS_ALLOWLIST` | Comma-separated hosts the SecretBroker may substitute `secret://` refs for on webhook egress; a non-allowlisted host gets the literal ref. Empty ⇒ never substitute. |
| `FEDERATION_DISABLED` | Turns off federation ingress + the pull worker. |
| `MAIDAN_DB_STATEMENT_TIMEOUT_MS` | Per-connection Postgres statement timeout (default 30 s) — caps a runaway query. |
| `MAIDAN_DB_IDLE_IN_TRANSACTION_TIMEOUT_MS` | Ends a connection idle inside an open transaction (default 60 s), so it cannot hold back vacuum. |
| `MAIDAN_MAX_CONCURRENT_REQUESTS` | In-flight HTTP request ceiling (default 1024, `0` off); past it, an immediate `503` with `Retry-After`. Lower it to protect the database under a spike. Gauge `maidan_http_in_flight_requests`, counter `maidan_http_shed_total`. |
| `MAIDAN_MAX_WS_CONNECTIONS` | Ceiling on live `/ws/subscribe` connections (default 10 000; past it, 503). Gauges: `maidan_ws_connections`, `maidan_mcp_streamable_sessions`. |
| `MAIDAN_DB_LOCK_TIMEOUT_MS` | Fails a statement waiting on a lock after 10 s by default; migrations exempt themselves. |
| Opt-in workers: `MAIDAN_SCHEDULER_TICK_SECS`, `MAIDAN_WAIT_SWEEP_TICK_SECS`, `MAIDAN_DIGEST_TICK_SECS`, `MAIDAN_MAIL_WORKER_TICK_SECS`, `MAIDAN_RETENTION_SWEEP_SECS` | Unset ⇒ the worker never starts. Unset one to stop that background activity (scheduled tasks / wait escalations / digests / mail / retention pruning). |
| `MAIDAN_RETENTION_*_DAYS` (events/audit/deliveries) | With the retention sweeper on, per-table age cutoffs; the event log is floored at the min at-least-once cursor so a lagging consumer never loses an undelivered event. |

Federation peer secrets and the secret store share the `FEDERATION_ENCRYPTION_KEY`
keyring; rotate with `FEDERATION_DECRYPT_KEYS`. See
[Production.md](Production.md) for the full config surface.

## Load & soak testing

`scripts/loadgen.sh` drives concurrent REST traffic (post message / read thread
/ search) at the server and prints per-op latency percentiles (p50/p95/p99) +
throughput — the baseline the rest of Arc D's optimizations are measured
against. The measurement is the `#[ignore]`d `load_baseline` test
(`crates/maidan-server/tests/loadgen.rs`), so it never runs as a pass/fail CI
gate (a hard latency floor would flake across runner hardware); the percentile
math is pure and unit-tested and *does* run in CI.

```sh
# in-process server (SQLite), defaults (8 workers × 50 iterations):
scripts/loadgen.sh

# tune concurrency + switch to a timed soak:
MAIDAN_LOADGEN_CONCURRENCY=32 MAIDAN_LOADGEN_DURATION_SECS=60 scripts/loadgen.sh

# point at a live/scaled deployment (bring your own ids + bearer):
MAIDAN_LOADGEN_URL=http://localhost:8080 \
  MAIDAN_LOADGEN_BEARER=<token> \
  MAIDAN_LOADGEN_IDS='<workspace>|<channel>|<thread>|<member>' \
  scripts/loadgen.sh
```

The report is one row per op kind with `count/min/mean/p50/p95/p99/max` (ms) and
an overall `ops/s`. Capture a baseline before an Arc D optimization and re-run
after to show the change.

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
- `scale-out smoke` — ~9m (required as of the `maidan-scale-1.0` gate)
- `promtool (alert rules)` — ~10s (required as)
- `otlp smoke` — ~9m (required as)

If anything goes red, fix on the branch and push again. The most
common failures and fixes are in "Debugging CI" below.

### 4a. Address the CodeRabbit review

CodeRabbit reviews every non-draft PR to `main` and reviews new pushes
incrementally (settings in `.coderabbit.yaml` at the repo root). Before
merging, address every comment it leaves, including those it lists
outside the diff in its review body: fix it, or reply with the
reason it does not apply. It is advisory, not a required check, so a
wrong comment is answered, not obeyed. `@coderabbitai review` asks for
a fresh review.

If no review arrives (rate limit or tool failure), ask with
`@coderabbitai review`. CodeRabbit is advisory, so a missing review does
not block a merge the 8 checks allow.

### 5. Merge

Merge only when the 8 required checks are green and every CodeRabbit
comment is addressed.

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

   The **container images** are also keyless-signed (`v158.0.0`): the
   `sign-images` job resolves each pushed tag to its immutable index digest and
   `cosign sign`s it. Verify (and enforce in an admission controller —
   Kyverno/Sigstore policy):

   ```sh
   cosign verify ghcr.io/david-engelmann/maidan-server:v0.X.0 \
     --certificate-identity-regexp '^https://github.com/david-engelmann/maidan' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com
   # same for ghcr.io/david-engelmann/maidan-postgres:v0.X.0
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

Run `cargo nextest run --workspace --lib --bins --profile ci` locally with
the same toolchain (`cargo test --workspace --lib --bins` runs the same
tests without nextest). The job uploads a `junit-unit` artifact naming each
failure. The toolchain pin is in `rust-toolchain.toml`; if a deep
transitive dep needs a newer rustc, bump the pin.

### `integration (testcontainers)` fails

Run `cargo nextest run --workspace --tests --profile ci` locally with
Docker running; the job uploads a `junit-integration` artifact. The `ci`
profile in `.config/nextest.toml` kills a test after 3 minutes and retries
only the quarantined tests listed there; a quarantined test that passes on a
retry is reported as `FLAKY`, not hidden. Common failures:

- "syntax error at or near `(`": a migration uses syntax that the
  testcontainer's Postgres major doesn't support. Verify the test is
  pinned to `pgvector/pgvector:pg17` (not `postgres:17-alpine`); the
  pg17 image supports everything pg16 supports plus the `vector`
  extension.
- "cannot DELETE from contentless fts5 table": the FTS5 schema was
  reverted to `content=''`. It must stay non-contentless.
- "the container failed to start with a Docker daemon running": Docker
  answered, so the test does not skip. The image could not be pulled or its
  readiness message never came; the error after the colon says which. A
  test skips (printing "skipping: no Docker daemon") only when no daemon
  answers, which is not the case on CI.
- `s3_roundtrip` or `s3_multipart` fails at "start the S3 container": the
  image in `crates/maidan-artifacts/tests/common/mod.rs` could not be pulled
  or did not log `API:`. These tests skip only when no Docker daemon answers,
  so a pull failure fails them instead of passing with `s3.rs` untested. Keep
  the image pinned to the digest `compose.yaml` uses.

### `loom` models fail

The loom models check every interleaving of the sharded bus and the presence
hub with [loom](https://docs.rs/loom). No CI job runs them yet; run them
locally after touching either:

```bash
cargo test -p maidan-bus --features loom --release --lib loom
cargo test -p maidan-server --features loom --release --lib loom
```

In the test build the `loom` feature swaps the locks for loom's, so it only
builds the models (the normal tests are compiled out). To see the failing interleaving, rerun
the failing model with `LOOM_LOG=trace LOOM_LOCATION=1`.

### `tla` specs fail

No CI job runs these yet. `scripts/tla.sh` model-checks the TLA+ specs in `specs/tla` with TLC (pinned
`tla2tools.jar`, Java 21); run it after touching claims or the event log. Each spec has a
passing config and one that turns off a mechanism (`ClaimNoReset.cfg`,
`EventLogUnordered.cfg`), where TLC must find the named invariant violated.
A failure prints a counterexample trace that breaks the invariant.

### NOTIFY floor simulation fails

`the_floor_delivers_every_committed_event_under_faults` in `maidan-bus`
prints the failing seed and its last steps. Replay it with the full trace:

```bash
MAIDAN_SIM_SEED=<seed> cargo test -p maidan-bus --lib notify_floor::sim -- --nocapture
```

`MAIDAN_SIM_SEEDS=5000` runs more seeds.

### `coverage (llvm-cov)` fails

The job runs the whole suite under `cargo llvm-cov nextest` and then
`scripts/coverage-floors.py`, which fails when the workspace or any crate is
under its line-coverage floor in `.config/coverage-floors.toml`, when a crate
has no floor, or when a floor names a crate that no longer exists. It is not a
required check.

- Reproduce locally (Docker running, so the Postgres suites count):

  ```sh
  cargo llvm-cov clean --workspace
  cargo llvm-cov nextest --workspace --profile ci --no-report
  cargo llvm-cov report --lcov --output-path lcov.info
  python3 scripts/coverage-floors.py lcov.info
  ```

  The same table is in the job's step summary, and `lcov.info` is in its
  `coverage` artifact. Skip the `clean` and the report also counts test
  binaries left over from an older build, whose lines show as uncovered: a
  crate far under its floor with more lines than its source has is that.
- A crate under its floor lost tested lines: add tests, or say in the PR why
  that code no longer needs them and lower the floor there.
- A new crate needs a floor in the same PR. Take its measured coverage from
  the job, subtract one point and round down to the half point.
- Raise a floor when a crate's coverage rises, the same way, citing the run.

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
   `after_id` until the frame stops.
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

### Authorization-denial troubleshooting (`v410.0.0`)

1. Scrape `maidan_authorization_decisions_total`. Its labels are closed
   vocabularies: `surface` (`rest|mcp`), capability `action`, `outcome`, and
   resource *kind*. Principal, workspace, subject, and future delegation grant
   IDs never become metric labels.
2. `MaidanAuthorizationDenialsElevated` fires above one denial/second for five
   minutes. Split the counter by `surface` and `action` to distinguish a stale
   integration grant from broad credential probing.
3. Detail warnings are content-free and sampled 1-in-64. They carry identity
   and resource IDs for correlation, but never request/response bodies,
   prompts, messages, tool arguments, secrets, or provider payloads.
4. Denials are not written to `maidan_audit`; this lane is aggregate
   observability and preserves the bounded-write decision from Cluster 182.

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
  at the `maidan-scale-1.0` gate), and `promtool (alert
  rules)` + `otlp smoke` (promoted).
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
