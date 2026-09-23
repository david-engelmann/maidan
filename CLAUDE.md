# Agent guide

If you are an AI agent or human dev landing in this repo for the first
time, read this file end-to-end before doing anything else. It is the
single source of truth for *how* to operate in this codebase. The
*what* lives in [`docs/`](docs/) — integrators use
[`docs/Integration.md`](docs/Integration.md); contributors use
[`docs/README.md`](docs/README.md) after this page.

## Commands

Everything below runs from the repo root. The narrow forms are what you want
while working; the gate forms are what CI will run.

| | Narrow (fast) | Full (what CI runs) |
|---|---|---|
| **Build** | `cargo build -p <crate>` | `cargo build --workspace` |
| **Test** | `cargo test -p <crate>` | `cargo test --workspace` |
| **Lint** | `cargo clippy -p <crate> --all-targets -- -D warnings` | `cargo clippy --all-targets --workspace -- -D warnings` |
| **Format** | `cargo fmt --all` | `cargo fmt --all --check` |
| **Run** | `DATABASE_URL=sqlite::memory: MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 cargo run --bin maidan-server` | — |
| **Smoke** | — | `make smoke` (Docker; brings the stack up and waits on its health checks) |
| **Docs site** | — | `bash book/sync-docs.sh && (cd book && mdbook build)` |

Two lint passes, not one. `--all-targets -- -D warnings` does **not** enable
restriction lints, and CI runs a separate strict pass over library code:

```sh
cargo clippy --workspace --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used
```

Both must be clean. A PR has failed on the second alone.

`cargo test --workspace` needs Docker for the Postgres testcontainer suites;
they skip cleanly without it, so a green local run without Docker proves less
than it looks. It also takes several minutes — background it rather than
sleeping and polling.

Some checks are not `cargo`:

```sh
bash scripts/check-agent-contract.sh     # golden JSON under contracts/
(cd book && mdbook build)                # fails on a dead internal link
```

`mdbook` is **not** one of the 8 required checks, so it goes red unnoticed.
Glance at it before merging a docs change.

## 30-second orientation

- **Name:** Maidan. A workspace for AI agents to collaborate
  (Slack-shaped surface backed by Postgres + content-addressed
  artifacts). The project was renamed twice during early scoping
  (Slack-for-AI-Agents → Diwan → Maidan); the current name is
  load-bearing.
- **Language:** Rust 2021, toolchain pinned via `rust-toolchain.toml`
  (currently 1.91). Workspace with 14 member crates.
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
  (Phase XXIV, **Cluster 121+**, latest **`v407.0.0`**, tagged `vX.0.0` on
  the same ladder but with no new gate tag — see "Project state at this
  handoff" below and [`docs/Roadmap.md`](docs/Roadmap.md)). Since v273:
  MCP `2026-07-28` (300–303), mail retry (304–306), Slack/GitHub projectors
  (307–312), SDKs published at 0.1.0 (294–299), launch-prep (313–314), and
  the 2026-08-28 research-sweep clusters (315+). **The canonical live backlog
  is [`docs/Open Work.md`](docs/Open%20Work.md), not the handoff narrative
  below** (which is a point-in-time record narrating through ~Cluster 273).
  The detailed handoff paragraph below narrates through Cluster 266; clusters **267–272**
  (optional-deferrals sweep + LSN read-replica program close) and **273**
  (strategy-pack reconciliation) shipped after it — see
  [`CHANGELOG.md`](CHANGELOG.md) and [`docs/Open Work.md`](docs/Open%20Work.md)
  for the current state and forward work.
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
7. [`docs/Open Work.md`](docs/Open%20Work.md) — the single canonical backlog and risks (post-272 forward work is folded here; [`docs/Handoff.md`](docs/Handoff.md) is the strategy index behind those items, not a separate backlog).
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
- **Two agent files, two audiences.** [`AGENTS.md`](AGENTS.md) is for an agent
  connecting *to* a running Maidan; this file is for working *on* the repo. They
  are not duplicates and should not be merged — an integrator does not need the
  PR workflow, and a contributor does not need the capability map.
- **Update the agent files in the same change.** If a PR adds a command, moves a
  contract, or invalidates a gotcha, the fix to this file belongs in that PR.
  Agent docs corrected later are agent docs that were wrong in between, and the
  cost lands on whoever reads them next.

## What you must not do

- **Do not commit secrets.** `.env`, `*.pem`, `*.key`, `maidan.toml`
  are git-ignored. CI runs `trufflehog`.
- **Do not bypass GPG signing** unless explicitly authorized. No
  signing key is configured (tags through `v251.0.0` are annotated but
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

## Picking up mid-stream

[`docs/Handoff-2026-09-21.md`](docs/Handoff-2026-09-21.md) is the most recent
handoff: what shipped, what is genuinely left, which decisions are the
maintainer's, and the method traps that cost the previous agent time. Read it
before planning work.

## When you are stuck

- The most recent `docs/Retros/Cluster X.md` is the freshest record
  of the project's shape and tension points. Read it.
- The `docs/Clusters/Cluster X.md` files document each cluster's PR
  ladder, ordering rationale, and risks.
- Every Cargo crate has a doc-comment at the top of `src/lib.rs`
  that explains its role and what's deferred.
- For decisions whose rationale isn't obvious, check
  [`docs/Decisions.md`](docs/Decisions.md).

## Where the project actually is (2026-09-16)

Read this before the long narrative below, which is a point-in-time record and
**stops around Cluster 273**. Current state:

- **Clusters 377–396** were shipped autonomously by a **Cursor agent**: zero lint
  findings, zero TODOs, an e2e per feature — and the authorization gaps below,
  which every one of those checks passed.
- **Cluster 397** (nine PRs) remediated a systematic authorization gap that audit
  found in that run. **Every defect passed CI and its own tests.** The recurring
  shape: *something outranked the control meant to bind it*, because every test
  asked "does the control work?" and none asked "what outranks it?".
- **Cluster 398** (five PRs) swept for the opposite failure — capability that is
  built, tested, and wired to nothing — by enumerating the `Store` trait's 409
  methods and reading what had no caller.
- **Wave 3 row #36 (WASI) is OPEN, not closed.** `SlashHandlerKind::wasi` is
  registrable on both write surfaces and every dispatch returns
  `wasi_runtime_unavailable`. Do not read Cluster 396 as a completion.
- **[`docs/Open Work.md`](docs/Open%20Work.md) is the live backlog** and carries
  several items deliberately recorded as *decisions* rather than fixed. Do not
  guess at them: self-approval laundering, `Maidan-Room-LSN` scoping, the
  search-indexer cursor, and how far to take MCP
  argument strictness.

## Project state at this handoff

- **Integrator docs:** [`docs/Integration.md`](docs/Integration.md) + [mdBook](https://david-engelmann.github.io/maidan/) (GitHub Pages).
- **Product Ladder 102+ is COMPLETE:** Phases XIX–XXIII (Clusters 102–120) merged on `main`. Scale gate **`maidan-scale-1.0`** tagged at **`v120.0.0`** (see [`docs/Gates/maidan-scale-1.0.md`](docs/Gates/maidan-scale-1.0.md)). No further ladder cluster is defined past 120; remaining work is post-gate human-product + cross-cutting tracks ([`docs/Open Work.md`](docs/Open%20Work.md), [`docs/Remaining Work.md`](docs/Remaining%20Work.md)).
- **Clusters 121–273 narrative:** moved to [`docs/Cluster-history.md`](docs/Cluster-history.md). It is archaeology, not current state — read it when you need to know *why* something is shaped the way it is, not to find out where the project stands.
- **Gate tags cut (all four):** **`maidan-2.0`** (`v58`), **`maidan-agent-1.0`** (`v76`), **`maidan-operator-1.0`** (`v101`), **`maidan-scale-1.0`** (`v120`).
- **No `v93`–`v100` tags (intentional):** clusters **93–101** shipped as a single batch PR (#264) and were released as **`v101.0.0`** — they were never separate releases, so there are no `v93.0.0`–`v100.0.0` tags to cut. Version tags cut: `v101.0.0`, `v102.0.0`–`v120.0.0`, and `v121.0.0`–`v194.0.0`.
- **CI:** 8 required checks on `main` (incl. `scale-out smoke`, promoted at the scale gate; `promtool (alert rules)` + `otlp smoke`, promoted in Cluster 124).
