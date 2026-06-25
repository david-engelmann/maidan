# Agent guide

If you are an AI agent or human dev landing in this repo for the first
time, read this file end-to-end before doing anything else. It is the
single source of truth for *how* to operate in this codebase. The
*what* lives in [`docs/`](docs/) — integrators use
[`docs/Integration.md`](docs/Integration.md); contributors use
[`docs/README.md`](docs/README.md) after this page.

## 30-second orientation

- **Name:** Maidan. A workspace for AI agents to collaborate
  (Slack-shaped surface backed by Postgres + content-addressed
  artifacts). The project was renamed twice during early scoping
  (Slack-for-AI-Agents → Diwan → Maidan); the current name is
  load-bearing.
- **Language:** Rust 2021, toolchain pinned via `rust-toolchain.toml`
  (currently 1.91). Workspace with 13 member crates.
- **Owner:** `david-engelmann`. Solo maintainer. Squash-merge only;
  admin-merge is the standard workflow (see
  [`docs/Operations.md`](docs/Operations.md)).
- **Release cadence:** work ships in clusters — the initial A–H + 1.0
  arc (`v0.X.Y` → `v1.0.0`), then a numbered product ladder (1–120,
  tagged `vX.0.0`). Every cluster closes with a mandatory retro PR and
  a tag. Current state: **Product Ladder 102+ is complete** — Phases
  XIX–XXIII (Clusters 102–120) closed on `main`; scale gate
  **`maidan-scale-1.0`** at **`v120.0.0`**. No further *ladder* cluster
  is defined past 120; subsequent clusters are **post-gate hardening**
  (Phase XXIV, **Cluster 121+**, latest **`v136.0.0`**, tagged `vX.0.0` on
  the same ladder but with no new gate tag — see "Project state at this
  handoff" below and [`docs/Roadmap.md`](docs/Roadmap.md)).
- **CI:** GitHub Actions, 8 required-status-checks on `main`
  (`lint`, `secrets scan`, `unit tests`, `integration
  (testcontainers)`, `docker compose smoke`, `scale-out smoke`,
  `promtool (alert rules)`, `otlp smoke`). Every PR runs all 8.
  (`scale-out smoke` was promoted at the `maidan-scale-1.0` gate, Cluster
  120; `promtool (alert rules)` + `otlp smoke` promoted in Cluster 124.)

## Read order

**External integrators (not editing this repo):** [`AGENTS.md`](AGENTS.md) →
[`docs/Integration.md`](docs/Integration.md) → published
[mdBook](https://david-engelmann.github.io/maidan/) — skip `docs/Clusters/`.

**Repo contributors:**

1. **This file** — operating manual.
2. [`docs/README.md`](docs/README.md) — doc index.
3. [`docs/Architecture.md`](docs/Architecture.md) — components and data flow.
4. [`docs/Capabilities.md`](docs/Capabilities.md) — what ships in which release.
5. [`docs/Decisions.md`](docs/Decisions.md) — load-bearing ADRs.
6. [`docs/Operations.md`](docs/Operations.md) — PR flow, CI, releases.
7. [`docs/Open Work.md`](docs/Open%20Work.md) — backlog and risks.
8. [`docs/Roadmap.md`](docs/Roadmap.md) / [`docs/Retros/`](docs/Retros/) — when doing cluster work.

## The cluster model in one paragraph

Work is sliced into **clusters** (A through H plus 1.0). Each cluster
delivers a coherent capability (`v0.0.1` foundation, `v0.1.0` routing
+ bus + MCP, `v0.2.0` search, etc.). Within a cluster, work is a
small numbered sequence of PRs (C.1, C.2, …). Every cluster closes
with a `[X.retro]` PR that writes `docs/Retros/Cluster X.md`,
prepends a new section to [`docs/Capabilities.md`](docs/Capabilities.md),
adds a `[v0.X.0]` section to [`CHANGELOG.md`](CHANGELOG.md), refreshes
[`docs/Architecture.md`](docs/Architecture.md) and the "Current
cluster" pointer in [`docs/Roadmap.md`](docs/Roadmap.md), then the
maintainer tags `v0.X.0` and pushes — which triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml). The
retro is mandatory. The tag does not get cut without it.

## PR workflow (the short version)

1. Open a GitHub Issue from the relevant template *or* link an
   existing cluster-phase issue (each cluster's plan in
   `docs/Clusters/Cluster X.md` lists the issues).
2. Branch from `main`: `<kind>/<scope>-<slug>` per
   [`docs/Conventions.md`](docs/Conventions.md). Examples:
   `feat/maidan-search`, `ci/release-darwin-x86`, `docs/cluster-c-retro`.
3. Develop on the branch. Locally run `cargo fmt --check`,
   `cargo clippy --all-targets --workspace -- -D warnings`, and
   the relevant test target (`cargo test -p <crate>`).
4. Commit with a Conventional Commits title (`feat(scope):`,
   `chore:`, `ci:`, `docs(retro):`).
5. `git push -u origin <branch>` and open the PR with `gh pr create`.
   The body **must** include the PR-level retro section per
   [`docs/Conventions.md`](docs/Conventions.md).
6. Wait for the 8 required CI jobs to pass. Use `gh pr checks <num>`
   or arm a Monitor.
7. Merge with `gh pr merge <num> -R david-engelmann/maidan --squash
   --admin --delete-branch`. The `--admin` flag is intentional and
   authorized — see [`docs/Decisions.md`](docs/Decisions.md) entry
   "Admin-merge instead of local-first push".
8. Sync local main: `git checkout main && git pull --ff-only && git
   branch -d <branch>`.

The full version is in [`docs/Operations.md`](docs/Operations.md).

## Test conventions you must know

- **Postgres testcontainers run against `pgvector/pgvector:pg17`**,
  not stock `postgres:11` (the default). Migration 0003 needs the
  `vector` extension. Pattern:

  ```rust
  use testcontainers::{runners::AsyncRunner, ImageExt};
  use testcontainers_modules::postgres::Postgres;

  let container = match Postgres::default()
      .with_name("pgvector/pgvector")
      .with_tag("pg17")
      .start()
      .await
  {
      Ok(c) => c,
      Err(err) => {
          eprintln!("skipping: docker unavailable ({err})");
          return;
      }
  };
  ```

- **Postgres tests skip gracefully if Docker is unavailable** —
  every integration test that uses testcontainers wraps `.start()`
  in a `match` with `eprintln!` + `return` on Err. Do not panic; CI
  for fork PRs may run without Docker.
- **SQLite tests use `sqlite::memory:`** with `PRAGMA foreign_keys =
  ON` explicitly turned on (off by default in SQLite).
- **Shared assertions go in `tests/common/mod.rs`**. Each test crate
  in the workspace that needs the pattern has its own copy
  (`maidan-store/tests/common/mod.rs`, `maidan-search/tests/common/mod.rs`).
  Both backends in each crate exercise the same suite from `common`.
- **Test names are descriptive sentences**, not action_under_test
  (`semantic_search_orders_by_cosine_distance`, not
  `test_semantic`).
- **No `tokio::sync::Notify::notify_waiters()` for cross-task
  signaling between a producer and a poller.** It only wakes
  *current* waiters. Use a polling loop instead — see
  `LoggingHandler::wait_for` in
  [`crates/maidan-search/src/indexer.rs`](crates/maidan-search/src/indexer.rs).

## Editing gotchas you must know

- **`Edit` requires `Read` first** for any file you intend to edit.
  This is enforced; a second `Edit` against a freshly-written file
  may fail if a linter (`cargo fmt`) touched it in between — re-Read
  the relevant range.
- **`cargo fmt` rewrites files**. It will reorder imports
  alphabetically and shift line breaks. After `cargo fmt && cargo
  fmt --check`, expect a notification that tracked files were
  modified by the linter — don't revert.
- **`Bash sed -i ''` for in-place edits on macOS** needs a backup
  extension argument: `sed -i.bak '...' file && rm file.bak`. Always
  clean up `.bak` after the substitution.
- **`Bash` auto-backgrounds long commands**. `cargo test` for full
  workspace can take several minutes; use the `run_in_background`
  parameter and the task notification, or `Monitor` for streamed
  results. Don't sleep-and-poll.

## Conventions that are not optional

- **No comments that restate code.** Only comment *why*, not *what*.
- **No `unwrap()` in library code** (`crates/maidan-*/src/`). Tests
  may unwrap freely.
- **`thiserror` for library errors**, `anyhow` only at binary
  boundaries.
- **`tracing` for logging** — no `println!` in library code.
- **Path deps inside the workspace** are fine and pinned via
  `publish = false` on every member crate (workspace-level
  `publish.workspace = true` inheritance). Don't change this without
  reading the `cargo-deny` decision in
  [`docs/Decisions.md`](docs/Decisions.md).
- **Squash-merge only**. Every PR's body becomes the squash commit's
  body — the PR-level retro lives there too.

## What you must not do

- **Do not commit secrets.** `.env`, `*.pem`, `*.key`, `maidan.toml`
  are git-ignored. CI runs `trufflehog`.
- **Do not bypass GPG signing** unless explicitly authorized. No
  signing key is configured (tags through `v136.0.0` are annotated but
  unsigned); annotated unsigned tags are acceptable until a key is set
  up.
- **Do not push to `main` directly.** Branch protection blocks it;
  even admins must PR.
- **Do not skip required CI checks** without explicit user
  authorization. Admin-merge with red CI is bypassing required-
  status-checks; only do it when the user has acknowledged the
  reason and authorized.
- **Do not introduce backwards-compatibility shims pre-1.0.** We
  rename, delete, and refactor freely until `v1.0.0` ships.

## When you are stuck

- The most recent `docs/Retros/Cluster X.md` is the freshest record
  of the project's shape and tension points. Read it.
- The `docs/Clusters/Cluster X.md` files document each cluster's PR
  ladder, ordering rationale, and risks.
- Every Cargo crate has a doc-comment at the top of `src/lib.rs`
  that explains its role and what's deferred.
- For decisions whose rationale isn't obvious, check
  [`docs/Decisions.md`](docs/Decisions.md).

## Project state at this handoff

- **Integrator docs:** [`docs/Integration.md`](docs/Integration.md) + [mdBook](https://david-engelmann.github.io/maidan/) (GitHub Pages).
- **Product Ladder 102+ is COMPLETE:** Phases XIX–XXIII (Clusters 102–120) merged on `main`. Scale gate **`maidan-scale-1.0`** tagged at **`v120.0.0`** (see [`docs/Gates/maidan-scale-1.0.md`](docs/Gates/maidan-scale-1.0.md)). No further ladder cluster is defined past 120; remaining work is post-gate human-product + cross-cutting tracks ([`docs/Open Work.md`](docs/Open%20Work.md), [`docs/Remaining Work.md`](docs/Remaining%20Work.md)).
- **Post-gate hardening (Phase XXIV, Cluster 121+):** opportunistic backlog burn-down tagged on the same `vX.0.0` ladder, no new gate tag. **Cluster 121** (`v121.0.0`) closed the OpenAPI-wide capability map in CI (Cluster 69 deferral) and extended the SLO dashboards/alerts to the Cluster 116 indexer metrics. **Cluster 122** (`v122.0.0`) added a `promtool (alert rules)` CI job that executes the SLO PromQL (it caught a `$value`-rendering bug in `MaidanIndexerQueueSaturated`) and corrected the OTLP-export status (shipped in Cluster 89, not open). **Cluster 123** (`v123.0.0`) added an `otlp smoke` CI job + `otlp` compose profile that proves OTLP traces + metrics reach a real collector end-to-end. **Cluster 124** (`v124.0.0`) consolidated the SLO-rule validators (one script) and promoted `promtool (alert rules)` + `otlp smoke` to required checks (**8 required** now). **Cluster 125** (`v125.0.0`) added opt-in at-least-once event delivery (`at_least_once` subscribe flag → cursor-driven reconcile over a stability horizon; default optimistic path unchanged). **Cluster 126** (`v126.0.0`) extended `at_least_once` to the MCP SSE transport (`/mcp/stream`). **Cluster 127** (`v127.0.0`) reconciled the backlog docs against code (struck ~11 phantom/already-shipped entries). **Cluster 128** (`v128.0.0`) hardened A2A delivery (client timeouts; push retry/backoff + metric; SSE error visibility). **Cluster 129** (`v129.0.0`) bounded the MCP streamable buffer + surfaced the outbox quarantine error + converted request-handler `unreachable!()` to typed errors. **Cluster 130** (`v130.0.0`) lifted observability/MCP test coverage (pure-parser extraction). **Cluster 131** (`v131.0.0`) closed delivery-unification as substantially-addressed (declined a risky storage merge). **Cluster 132** (`v132.0.0`) shipped `GET /operator/audit` (global cross-workspace audit, gated by the new `audit:read-global` capability), completing the 127–132 sweep. **UI track:** **Cluster 133** (`v133.0.0`) repaired the broken `/ui` write path (4 undefined JS refs) + added `tests/ui_js_contract.rs` (a CI guard for undefined-helper bugs — the `/ui` JS is otherwise untested, no browser in CI); feature clusters 134+ build on it. **Cluster 134** (`v134.0.0`) added emoji reactions in the `/ui` (over new `/ui/api/messages/:mid/reactions` routes reusing the tested handlers). **Cluster 135** (`v135.0.0`) added message pins in the `/ui` (per-message 📌 toggle over `/ui/api/threads/:tid/pins`). **Cluster 136** (`v136.0.0`) added group DMs in the `/ui` (a new "Group DMs" tab: open/list/read/post over new `/ui/api/.../group-dms` routes reusing the tested `group_dm::*` handlers; the conversation pane reads via the existing thread-messages route).
- **Gate tags cut (all four):** **`maidan-2.0`** (`v58`), **`maidan-agent-1.0`** (`v76`), **`maidan-operator-1.0`** (`v101`), **`maidan-scale-1.0`** (`v120`).
- **No `v93`–`v100` tags (intentional):** clusters **93–101** shipped as a single batch PR (#264) and were released as **`v101.0.0`** — they were never separate releases, so there are no `v93.0.0`–`v100.0.0` tags to cut. Version tags cut: `v101.0.0`, `v102.0.0`–`v120.0.0`, and `v121.0.0`–`v136.0.0`.
- **CI:** 8 required checks on `main` (incl. `scale-out smoke`, promoted at the scale gate; `promtool (alert rules)` + `otlp smoke`, promoted in Cluster 124).
