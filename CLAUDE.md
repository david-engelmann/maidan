# Agent guide

If you are an AI agent or a human landing in this repo for the first time, read
this file end to end before doing anything else. It says *how* to work in the
codebase. The *what* lives in [`docs/`](docs/): integrators start at
[`docs/Integration.md`](docs/Integration.md), contributors at
[`CONTRIBUTING.md`](CONTRIBUTING.md) and the [doc index](docs/README.md).

## Commands

Everything runs from the repo root. Use the narrow forms while working; the
full forms are what CI runs.

| | Narrow (fast) | Full (what CI runs) |
|---|---|---|
| **Build** | `cargo build -p <crate>` | `cargo build --workspace` |
| **Test** | `cargo test -p <crate>` | `cargo nextest run --workspace --lib --bins --profile ci` and `cargo nextest run --workspace --tests --profile ci` |
| **Lint** | `cargo clippy -p <crate> --all-targets -- -D warnings` | `cargo clippy --all-targets --workspace -- -D warnings`, then the strict pass below |
| **Format** | `cargo fmt --all` | `cargo fmt --all --check` |
| **Run** | `MAIDAN_ALLOW_INSECURE_DEV_KEK=1 DATABASE_URL=sqlite::memory: MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 cargo run --bin maidan-server` | — |
| **Smoke** | — | `make smoke` (Docker; brings the stack up and waits on its health checks) |
| **Mutants** | `cargo mutants -p <crate> --shard 0/20` | nightly: `scripts/mutants.sh plan`, then the shards ([`docs/Conventions.md`](docs/Conventions.md), "Nightly mutation testing") |
| **Fuzz** | `cd fuzz && cargo +nightly fuzz run <target> -- -max_total_time=60` | nightly, in `fuzz/`, for each target that `cargo +nightly fuzz list` prints: `cargo +nightly fuzz run --target x86_64-unknown-linux-gnu <target> corpus/<target> seeds/<target> -- -dict=json.dict -max_total_time=300` (`seeds/<target>` only where it exists; [`docs/Conventions.md`](docs/Conventions.md), "Nightly fuzzing") |
| **Docs site** | — | `mdbook-mermaid install book && bash book/sync-docs.sh && mdbook build book && ./scripts/check-docs-presentation.sh` |

The server refuses to start without a content key: set `MAIDAN_CONTENT_KEK`, or
`MAIDAN_ALLOW_INSECURE_DEV_KEK=1` for local work only.

**Two lint passes, not one.** `--all-targets -- -D warnings` does not enable
restriction lints, and CI runs a separate strict pass over library code:

```sh
cargo clippy --workspace --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used
```

Both must be clean; PRs have failed on the second alone. `make ci` runs both.

**Tests need Docker for the Postgres suites.** A testcontainers test skips only
when no Docker daemon answers; with Docker running, a start failure fails the
test. A green local run without Docker proves less than it looks. The full
suite takes several minutes: run it in the background, don't sleep and poll.

**Checks that are not `cargo`:**

```sh
bash scripts/check-agent-contract.sh     # golden JSON under contracts/
bash scripts/check-release-records.sh    # Capabilities, CHANGELOG, this file and the README agree on the release
bash scripts/check-deploy-pins.sh        # every deploy pin names the newest tagged release
bash scripts/osv-scan.sh                 # advisories in the lockfiles cargo-deny does not read (fuzz/, ui-tests/, sdk/)
cargo vet --locked                       # every crate in Cargo.lock audited or exempted (supply-chain/)
```

The docs build (the `mdbook` job) needs mdbook 0.4.40, mdbook-linkcheck 0.7.7
and mdbook-mermaid 0.14.1; it fails on a dead internal link. It is not a
required check, so it goes red unnoticed: look at it before merging a docs
change.

## Orientation

- **Maidan** is a workspace where AI agents and people collaborate on work: a
  Slack-shaped surface (workspaces, channels, threads, messages) with a task
  queue, reviews, approval gates and a hash-chained event log, backed by
  Postgres or SQLite and content-addressed artifacts. The name is load-bearing.
- **Rust 2021**, toolchain pinned in `rust-toolchain.toml` (1.91).
  Workspace with 15 member crates.
- **Owner:** `david-engelmann`, solo maintainer. Squash-merge only;
  admin-merge is the standard workflow ([`docs/Operations.md`](docs/Operations.md)).
- **CI:** GitHub Actions. Eight checks are required on `main`:
  `lint (fmt + clippy + deny)`, `secrets scan`, `unit tests`,
  `integration (testcontainers)`, `docker compose smoke`, `scale-out smoke`,
  `promtool (alert rules)` and `otlp smoke`. A code PR runs all eight; a
  docs-only PR skips the heavy jobs, which then report as passed. Other jobs
  (coverage, `mdbook`, the A2A TCK, the MCP Inspector, SDK interop, the PITR
  drill, `ui tests (playwright)`, `loom`, `tla`, the OSV scan, `cargo vet`,
  `open work`, `changelog (released sections)`, the SQLite backup drill) are
  not required.

## Current state (2026-10-01)

- **Releases:** latest **`v412.0.0`**. Release tags are cut when the
  maintainer chooses, not per cluster, so some clusters were never tagged
  (v23–26, v78–100, v311, v350–401, v403, v411); their work ships in the next
  tag. `main` is well ahead of `v412.0.0`. Tagging is the maintainer's call.
- **Do not point anyone at a deploy yet.** No deploy path runs a release with
  the week's cross-tenant fixes: the release pins name `v412.0.0`, and some
  paths run `dev`, `latest`, `v315.0.0` or `v0.0.1`. Open Work's **Before anyone
  deploys** lists the blockers; the pin bump waits for the next tag, and
  `scripts/check-deploy-pins.sh` (from #1156) says which lines to change.
- **The forward plan is the ranked list in
  [`docs/Open Work.md`](docs/Open%20Work.md).** What shipped is in
  [`docs/Capabilities.md`](docs/Capabilities.md) and [`CHANGELOG.md`](CHANGELOG.md).
  Everything else in `docs/` that looks like a plan is history.
- **Open Work and the Roadmap have one keeper.** A feature PR does not edit
  them, not even its own row: put what belongs there in the PR body, and the
  keeper reconciles after the merge. When each PR edited its own row, every PR
  conflicted with every other.
- **Decisions only the maintainer makes** are listed in Open Work under
  "Decisions pending the maintainer" (on 2026-10-01, only when to cut the next
  release and the launch). Don't guess at them. The ones he delegated are under
  "Recently decided" and in [`docs/Decisions.md`](docs/Decisions.md).

Working beside other agents (several PRs are usually open at once):

- **CHANGELOG:** put your entry at the end of `[Unreleased]`, immediately
  before the first `## [N.0.0]` line, or before a heading you have checked is
  above that line. An entry placed before a heading under a released version is
  filed in that release. `scripts/check-changelog-released.sh` fails when the
  newest tagged section gains an entry after its tag, and when `[Unreleased]`
  lists an entry twice. It does not check older sections.
- **Counters that two PRs can bump by the same line:** the `.route(` count
  pinned in `tenant_isolation_e2e`, the MCP tool count in `docs/Protocols.md`,
  `README.md` and `book/src/introduction.md`, and migration numbers. Git merges
  two identical edits into one, so after a rebase recount (`grep -o '\.route('
  crates/maidan-server/src/app.rs | wc -l`, which counts what the test counts)
  rather than trusting the number, and take
  the next free migration number on `main` plus the open PRs.
- **Contracts new code must satisfy:** a route in
  `contracts/http-operation-kinds.json`; a `MAIDAN_*` name in
  `crates/maidan-env/src/lib.rs`; `NewAuditEvent` needs a
  `scope: AuditScope`; a spawned task that writes uses
  `maidan_store::attribution::spawn`; a member id from a request goes through
  `Store::get_member_in(workspace, id)` (from #1152).
- **Merging:** an admin squash only when all eight required checks passed on
  the PR's exact head commit and every CodeRabbit comment has a fix or a reply,
  merged with `gh pr merge --squash --match-head-commit <sha>`; `gh pr checks` and even
  `headRefOid` can show the previous commit for a moment after a push. List the
  unanswered comments with the `gh api .../pulls/<N>/comments` query in
  [`docs/Conventions.md`](docs/Conventions.md) ("CodeRabbit").

Lessons that cost real time, and that still apply:

- **Something outranked the control meant to bind it.** Every authorization
  defect found in the 2026-09 audits passed CI and its own tests, because every
  test asked "does the control work?" and none asked "what outranks it?".
- **An optional id is an authorization bug until shown otherwise.**
  `if let Some(ws) = req.workspace_id { check(ws) }` let omitted fields mean
  "every tenant" (#1029, #1031, #1055, #1090). Default to the caller's scope,
  and give every multi-tenant fix a two-tenant test. `tenant_isolation_e2e`
  probes every route, MCP tool and stream with another tenant's token.
- **Run the examples, not just the tests.** A recipe run found that
  `claim_next_thread` re-handed finished work (#1046) after every unit test had
  passed.
- **A job that cannot fail reports nothing.** Until 2026-09-30 no nightly
  mutation shard had tested a mutant (#1155), and until October no fuzz
  target had run in CI: the fuzz job failed every night before compiling one.
  `continue-on-error` showed both green. No nightly job has it now; a failing
  night is red. On Linux, a cargo-fuzz release binary needs `--target
  x86_64-unknown-linux-gnu` (it defaults to musl, which has no sanitizer).
- **A required check red at the same step on consecutive `main` commits is a
  break, not a flake.** `docker compose smoke` was red from #973 to #1005 while
  about 30 PRs were admin-merged over it.

## Read order

**Integrators (not editing this repo):** [`AGENTS.md`](AGENTS.md) →
[`docs/Integration.md`](docs/Integration.md) → the published
[docs site](https://david-engelmann.github.io/maidan/).

**Contributors:**

1. This file.
2. [`CONTRIBUTING.md`](CONTRIBUTING.md): branches, commits, the PR retro.
3. [`docs/Architecture.md`](docs/Architecture.md): components and data flow.
4. [`docs/Operations.md`](docs/Operations.md): CI, releases, debugging a red job.
5. [`docs/Open Work.md`](docs/Open%20Work.md): the ranked plan and the open decisions.
6. [`docs/Decisions.md`](docs/Decisions.md): before changing a decided area.
7. [`docs/Capabilities.md`](docs/Capabilities.md) and [`CHANGELOG.md`](CHANGELOG.md): what shipped.
8. `docs/Retros/`, `docs/Clusters/` and [`docs/Cluster-history.md`](docs/Cluster-history.md):
   only to learn *why* something is shaped the way it is.

## How work is sliced

Work ships in numbered **clusters**, each a coherent capability delivered as a
short sequence of PRs (413.1, 413.2, …). A cluster closes with a retro PR that
writes `docs/Retros/Cluster N.md`, prepends a source record to
`docs/Capabilities.md`, adds its entries to `CHANGELOG.md` under
`[Unreleased]`, and updates Open Work. Tagging is separate: when the maintainer
cuts a release, `scripts/check-release-records.sh` (the `release-record` job)
requires Capabilities, CHANGELOG, this file's "latest" line and the README pins
to agree on it.

## PR workflow

1. Branch from `main`: `<kind>/<scope>-<slug>`, where kind is `feat`, `fix`,
   `docs`, `test`, `ci`, `build`, `chore`, `refactor` or `perf`
   ([`CONTRIBUTING.md`](CONTRIBUTING.md)).
2. Before pushing: `cargo fmt --all --check`, both clippy passes, and the tests
   for what you touched (`make ci` covers fmt, both lints, deny and tests).
3. Commit with a Conventional Commits title (`feat(scope):`, `fix:`, `docs:`,
   `ci:`, `test:`).
4. `git push -u origin <branch>` and `gh pr create`. The PR body must include
   the "Retrospective (PR-level)" section (what was surprising, what got
   deferred, what we learned); the [PR template](.github/pull_request_template.md)
   has it.
5. Wait for the eight required checks (`gh pr checks <num>`). Answer every
   CodeRabbit comment: fix it, or reply with why it does not apply. CodeRabbit
   is advisory ([`.coderabbit.yaml`](.coderabbit.yaml)).
6. Merge with `gh pr merge <num> -R david-engelmann/maidan --squash --admin`.
   `--admin` is authorized (Decisions: "Admin-merge instead of local-first
   push") but it can merge over red checks, so it is only for green PRs.
   Don't pass `--delete-branch` while another PR is stacked on the branch: it
   closes the stacked PR for good.
7. The squash commit keeps the branch's commit messages, not the PR body, so the
   PR itself is where the retro lives.

The long version is in [`docs/Operations.md`](docs/Operations.md).

## Test conventions

- **Postgres testcontainers run `pgvector/pgvector:pg17`** (migration 0003
  needs the `vector` extension):

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
          maidan_store::test_support::docker::skip_start_failure(err).await;
          return;
      }
  };
  ```

  `skip_start_failure` returns only when no Docker daemon answers a ping; with
  Docker up, a start failure panics. Once the container is up, setup errors
  `expect`, never `.ok()?`: a skip there would hide a broken migration. The
  helper is behind `maidan-store`'s `test-support` feature; `maidan-artifacts`
  has its own copy in `tests/common`.
- **SQLite tests use `sqlite::memory:`** with `PRAGMA foreign_keys = ON`
  (off by default in SQLite).
- **Store behavior is tested on both backends** from one suite: a shared
  `run_suite(store: &dyn Store)` called by a `_sqlite` and a `_postgres` test.
  Shared helpers live in each crate's `tests/common/mod.rs` (`maidan-store`,
  `maidan-search`, `maidan-artifacts`).
- **Test names are sentences** (`semantic_search_orders_by_cosine_distance`,
  not `test_semantic`).
- **Don't use `tokio::sync::Notify::notify_waiters()`** between a producer and
  a poller: it wakes only current waiters. Poll instead (see
  `LoggingHandler::wait_for` in `crates/maidan-search/src/indexer.rs`).
- **Outbound workers share a per-host retry budget** (`retry_budget.rs`: 10
  retries at once, then 2 a second). A test that drives more retries than that
  at one mock host in one pass sees them deferred, not sent; give it its own
  `state.retry_budget` on a `ManualClock`.
- **A fix is checked by breaking it.** Revert the fix, confirm its test fails,
  restore it. Delete any `.proptest-regressions` file a mutated run leaves.

## Repo gotchas

- **A new migration is not picked up on its own.** Register it as an
  `include_str!` constant plus an `apply_postgres`/`apply_sqlite` call in
  `crates/maidan-store/src/migrate.rs`, for both backends
  (`tests/migration_register.rs` checks). `schema_parity` then checks both
  backends build the same tables, columns, keys and foreign keys.
- **A module for one backend only** goes in `POSTGRES_ONLY_MODULES` or
  `SQLITE_ONLY_MODULES` in `crates/maidan-store/tests/backend_parity.rs`.
- **A new column read by a shared row mapper** must be added to every `SELECT`
  and `RETURNING` that feeds it, across sibling modules.
- **A new route** needs an entry in `contracts/http-capability-map.json`, an
  OpenAPI path (`openapi_well_formed` fails on an undeclared path parameter or
  a dangling `$ref`), and, for a POST, PUT or PATCH, a body clause in
  `http_capability_matrix_e2e.rs`, or the extractor's 400 hides the 403.
  `tenant_isolation_e2e` then probes it with another tenant's token, and pins
  the total `.route(` count in `app.rs`: bump the number, and if the route is a
  live stream, add a probe in `stream_probes`.
- **A new route must be classified** `reads` or `changes` in
  `contracts/http-operation-kinds.json`. A GET that writes or a POST that only
  reads needs a `reason`, and a POST that only reads also goes in
  `auth::READ_ONLY_OPERATIONS`, or the request layer records it as a change.
- **A new `MAIDAN_*` variable** goes in `SERVER_ENV` (the server reads it) or
  `TOLERATED_ENV` (a script, SDK, test or build arg does) in
  `crates/maidan-env/src/lib.rs`, or boot refuses it as unknown.
  `env_registry_contract` checks every name the repo mentions.
- **Spawn with `maidan_store::attribution::spawn`**, not `tokio::spawn`, in
  any server or MCP module that is not a background worker: a plain spawn
  leaves the request's attribution scope and its writes read as nobody's.
  `attribution_scope_contract` scans every module and names the exceptions.
- **utoipa 4:** an `IntoParams` struct publishes its fields as *path*
  parameters unless it says `#[into_params(parameter_in = Query)]`; every type
  a schema names must be listed in `components(schemas(...))`; a field written
  as `maidan_types::X` becomes the dangling `$ref` `maidan_types.X`.
- **New v4 UUIDs fail `uuid_v7_contract`.** Entity ids use `Uuid::now_v7()`;
  credentials and random ids (tokens, session ids, trace ids) are allowlisted
  with a reason.
- **Two strings in this file are read by CI:** `Workspace with 15 member
  crates.` (`docs_numbers_contract`) and the `latest` release line
  (`check-release-records.sh`). Keep them when editing.
- **A dependency bump or a new crate needs `cargo vet`.** Every crate in the
  root `Cargo.lock` must be audited or exempted in `supply-chain/`, or the
  `cargo vet (root lockfile)` job goes red. Run `cargo vet`, then certify or
  exempt what it lists, and commit `supply-chain/` with the lockfile
  ([`docs/Conventions.md`](docs/Conventions.md), "Dependencies").
- **Cutting a release means bumping every deploy pin in one PR** (Helm prod
  values for both charts, the stack's twice: server and `maidan-postgres`;
  the quickstart compose file and Dockerfile's
  `MAIDAN_VERSION`, the prod kustomize overlay, the install commands in the
  README and `docs/Production.md`). The Dockerfile's tarball SHA-256s move
  with that version, but `scripts/check-deploy-pins.sh` does not check them:
  it checks `MAIDAN_VERSION` and the other pins it lists, and goes red (the
  `deploy pins` job) while any differs from the newest tagged CHANGELOG
  section. A change to `helm/maidan` also means repackaging
  `helm/maidan-stack/charts/maidan-0.1.0.tgz`
  (`helm package helm/maidan -d helm/maidan-stack/charts`), or the stack keeps
  rendering the old chart; `helm-template-smoke.sh` fails while they differ.
  Bumping the Chainguard MinIO digest in compose means bumping it in `k8s/`
  and `helm/maidan-stack/values.yaml` too; `check-deploy-contract.sh` (in the
  `lint` job) fails while they differ.
- **Splitting a large source file into a module directory** can break the
  `bootstrap compile-time strip` job (imports unused under
  `--no-default-features`) and `check-agent-contract.sh` (it greps paths).

## Harness gotchas (for agents)

- **`Edit` needs a fresh `Read`.** `cargo fmt` rewrites files; re-read before a
  second edit.
- **macOS `sed -i` needs a backup suffix:** `sed -i.bak '…' f && rm f.bak`.
- **The shell is zsh:** an unquoted `$VAR` holding several words is one
  argument. Use arrays.
- **Build caches fill the disk.** Build with `CARGO_INCREMENTAL=0` and
  `CARGO_PROFILE_DEV_DEBUG=0`; a shared target directory with incremental on
  grew past 120 GB. Worktrees sharing one target dir can link each other's
  crates: touch the sources before a local gate.
- **Never `git add -A` at the repo root** of a working checkout: it sweeps in
  untracked local files.

## Conventions that are not optional

- **Comments say why, never what.**
- **No `unwrap()` or `expect()` in library code** (`crates/maidan-*/src/`).
  Tests may unwrap.
- **`thiserror` for library errors,** `anyhow` only at binary boundaries.
- **`tracing` for logging;** no `println!` in library code.
- **Path dependencies inside the workspace** stay `publish = false`; read the
  `cargo-deny` entry in [`docs/Decisions.md`](docs/Decisions.md) before
  changing that.
- **Two agent files, two audiences.** [`AGENTS.md`](AGENTS.md) is for an agent
  connecting *to* a running Maidan; this file is for working *on* the repo.
  Don't merge them.
- **Update the agent files in the same change.** If a PR adds a command, moves a
  contract or invalidates a gotcha, fix this file in that PR.

## What you must not do

- **Don't commit secrets.** `.env`, `*.pem`, `*.key` and `maidan.toml` are
  git-ignored; CI runs `trufflehog`.
- **Don't push to `main`.** It is policy: branch protection does not apply to
  admins, so nothing technical stops you.
- **Don't merge over a red required check** without the maintainer's explicit
  go-ahead for that PR.
- **Don't add backwards-compatibility shims.** Until the public 1.0 launch gate
  (not the historical `v1.0.0` tag), rename, delete and refactor freely; there
  is no compatibility promise (Open Work, F-54).
- **Don't bypass GPG signing** unless told to. No signing key is configured;
  tags through `v412.0.0` are annotated and unsigned.
- **Don't cut release tags.** That is the maintainer's call.

## When you are stuck

- The newest `docs/Retros/Cluster N.md` is the freshest record of the
  project's shape and tension points.
- Each crate's `src/lib.rs` (`src/main.rs` for `maidan-cli`) opens with a doc
  comment on its role and what is deferred.
- For a decision whose reason isn't obvious, check
  [`docs/Decisions.md`](docs/Decisions.md).
