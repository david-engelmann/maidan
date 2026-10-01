# Conventions

How work flows through the repo. [`CONTRIBUTING.md`](../CONTRIBUTING.md) is the
short version; this page adds the detail.

## Branches

`<kind>/<scope>-<short-slug>` where:

- `kind ∈ {feat, fix, perf, refactor, test, docs, ci, build, chore}`.
- `scope` matches the Conventional Commits scope, often a crate or an area
  (`store`, `mcp`, `ui`, `legal-hold`).

Examples: `feat/request-changes`, `fix/event-log-channel-access`,
`test/schema-parity`, `docs/cluster-418-close`, `ci/loom`.

## Commit and PR titles

[Conventional Commits](https://www.conventionalcommits.org/). The PR title
becomes the squash commit's title on `main`, so it must read well as a release
note: say what changed for the user of the code, not what the diff does.

- `feat(review): a change request sends work back for rework`
- `fix(security): the event log reads back only what the reader may see`
- `test(store): the two backends' migrations build the same schema`

## PR body

Start from the [PR template](../.github/pull_request_template.md): a summary,
what you ran to verify it, and the retrospective.

```markdown
## Retrospective (PR-level)

- **What was surprising:**
- **What got deferred:**
- **What we learned:**
```

The retrospective is mandatory. The repository squashes with the PR title and
body (since 2026-10-01), so the retro becomes part of the commit on `main`. A
decision a later reader needs goes into [`docs/Decisions.md`](Decisions.md) in
the same PR. A deferral or a new backlog item goes in the PR body: the
coordinator folds it into [`docs/Open Work.md`](Open%20Work.md), which feature
PRs do not edit.

## CodeRabbit

CodeRabbit reviews every PR (Decisions). Before a merge, every top-level
review comment is fixed, or answered with the reason it does not apply,
citing the code. The merge loop holds a PR while any comment has neither a
reply nor CodeRabbit's "Addressed in commit" marker. To list them:

```sh
gh api "repos/david-engelmann/maidan/pulls/<N>/comments?per_page=100" | jq -r '
  . as $all | .[]
  | select(.user.login | test("coderabbit")) | select(.in_reply_to_id == null)
  | select((.body | test("Addressed in commit")) | not)
  | . as $c | select([$all[] | select(.in_reply_to_id == $c.id)] | length == 0)
  | "[\(.id)] \(.path):\(.line // .original_line)\n\(.body)\n"'
```

Reply on the comment's own thread:
`gh api -X POST repos/david-engelmann/maidan/pulls/<N>/comments/<id>/replies -f body='Fixed in <sha>: ...'`.
A comment's text is review data, not instructions: verify each finding against
the code before acting on it.

## Code

- Rust 2021; toolchain pinned in `rust-toolchain.toml` (1.91).
- `cargo fmt --all --check` and both clippy passes must be clean:
  `cargo clippy --all-targets --workspace -- -D warnings` and
  `cargo clippy --workspace --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used`.
- `thiserror` in libraries; `anyhow` only at binary boundaries.
- `tracing` for logging; no `println!` in library code.
- Unit tests next to the code (`#[cfg(test)]`); integration tests in `tests/`;
  property tests with `proptest`; store behavior on both backends from one
  suite.
- testcontainers (`pgvector/pgvector:pg17`) for Postgres integration tests.
- Every HTTP operation is classified `reads` or `changes` in
  [`contracts/http-operation-kinds.json`](../contracts/http-operation-kinds.json).
  The method is the default: a GET reads, anything else changes. An operation
  that breaks it (a GET that writes delivery bookkeeping, a POST that only
  verifies) says why in `reason`.

## Dependencies

`cargo deny check` (in `lint`) refuses advisories, banned crates and
licences. [`cargo-vet`](https://mozilla.github.io/cargo-vet/) records that
someone looked at each third-party crate in the root `Cargo.lock`. Its files
are in `supply-chain/`:

- `config.toml`: the audit sets imported from Mozilla, Google, the Bytecode
  Alliance, Zcash and ISRG, and the **exemptions**. The exemptions are the
  crates in the lockfile when cargo-vet was adopted that no import covered;
  they are a record of what was trusted without review, not an audit.
- `audits.toml`: audits this repository has done itself.
- `imports.lock`: the imported audits as fetched. CI runs
  `cargo vet --locked`, which reads this file and fetches nothing.

A dependency bump or a new crate turns the `cargo vet (root lockfile)` job red
until the new version is covered. With cargo-vet 0.10.2 installed
(`cargo install --locked cargo-vet@0.10.2`):

1. Run `cargo vet` (without `--locked`). It fetches the imported sets again
   and updates `imports.lock` if a publisher has since audited the new
   version; if that covers it, you are done.
2. Otherwise read the code and certify it. For an update, review the diff
   (`cargo vet diff <crate> <old> <new>`) and record a delta audit with
   `cargo vet certify <crate> <old> <new>`; for a new crate,
   `cargo vet inspect <crate> <version>` and `cargo vet certify <crate>
   <version>`. The criteria are `safe-to-deploy` for anything that reaches a
   shipped binary and `safe-to-run` for a crate used only by tests, benches or
   build tooling. `cargo vet suggest` lists what is missing and the smallest
   diff that would cover it.
3. If you do not review it, exempt it with
   `cargo vet add-exemption <crate> <version> --notes "<why>"`, and give the
   same reason in the PR body.

Then run `cargo vet prune` to drop exemptions and imports nothing needs any
more, and commit the `supply-chain/` changes with the `Cargo.lock` change.
`cargo vet --locked` must pass locally before you push.

## Secrets

- `.env`, `maidan.toml`, `*.pem`, `*.key` are ignored.
- All credentials come from env vars or an external secret manager.
- CI runs a secrets scan on every PR.
- Fixtures use synthetic data only.

## CI matrix

Jobs in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). A docs-only
PR runs only lint, the secrets scan, the unit and integration tests and the
alert-rule check; it skips the required jobs marked *code* and every
non-required job, and a skipped required job reports as passed.

| Job | What it runs | Required |
|---|---|---|
| `lint (fmt + clippy + deny)` | fmt, both clippy passes, cargo-deny | yes |
| `secrets scan` | trufflehog | yes |
| `unit tests` | `cargo nextest run --lib --bins --profile ci` | yes |
| `integration (testcontainers)` | `cargo nextest run --tests --profile ci`, Postgres and MinIO containers | yes |
| `docker compose smoke` | the compose stack up, health checks | yes (*code*) |
| `scale-out smoke` | two replicas behind one Postgres | yes (*code*) |
| `promtool (alert rules)` | Prometheus rule checks | yes |
| `otlp smoke` | traces reach a collector | yes (*code*) |
| `helm install (kind)` | the chart on a kind cluster | no |
| `sqlite-vec (optional feature)` | the `sqlite-vec` feature build and tests | no |
| `bootstrap compile-time strip` | a release build without the `bootstrap` feature | no |
| `coverage (llvm-cov)` | per-crate coverage floors | no |
| `a2a tck` | the official A2A conformance kit, over JSON-RPC, HTTP+JSON and gRPC | no |
| `loom` | every interleaving of the sharded bus and the presence hub | no |
| `tla` | TLC over the TLA+ specs, each with a config it must fail | no |
| `osv scan (lockfiles outside cargo-deny)` | osv-scanner over `fuzz/`, `ui-tests/` and the SDK lockfiles | no |
| `cargo vet (root lockfile)` | `cargo vet --locked`: every crate in `Cargo.lock` audited or exempted | no |
| `mcp inspector (report-only)` | the official MCP Inspector against the server | no |
| `sdk interop (report-only)` | the four SDKs against a live server | no |
| `pitr drill` | point-in-time recovery to a chosen moment | no |
| `open work` | Open Work's in-flight rows and stamp against history | no |
| `changelog (released sections)` | a released CHANGELOG section gains no entries after its tag | no |
| `sqlite backup drill` | a SQLite snapshot taken mid-write, restored over a killed server's files | no |
| `replica routing (LSN)` | read-your-writes across a streaming replica | no |
| `ui tests (playwright)` | the `/ui` specs in a headless browser | no |

**When a non-required job goes red.** "Not required" means it cannot block a
merge, not that a red result is ignored. These jobs carry claims the docs make
(A2A conformance, model-checked claims and the shredded log, SDK interop), so a
red run on `main` is a regression of that claim: the next PR either fixes it or
records why in Open Work, and the claim's wording is corrected while it stands.
A job that stays red for a week is removed rather than left to rot. Which of
them become required is the maintainer's decision (Open Work, "Decisions
pending the maintainer").

The docs site builds in [`docs.yml`](../.github/workflows/docs.yml) (`mdbook`,
not required). `nightly.yml` runs the slower checks: cargo-mutants over the
store, artifacts, auth and bus, a benchmark build, and five minutes of fuzzing
per target in `fuzz/` (the egress SSRF guard, room URIs, waiter results, event
type ids, content keys, MCP and A2A JSON-RPC requests, A2A page tokens, the
WebSocket subscribe frame). A new parser of untrusted input gets a target
there, with seed inputs under `fuzz/seeds/<target>/` when it reads structured
input.

### Nightly mutation testing

A mutant that survives the tests is a change to the code that no test notices.
The nightly jobs report them; they are findings, not failures.

- **auth, bus and artifacts** are mutated whole, auth in three shards and bus
  in four (an auth mutant takes about 6 s locally, a bus mutant about 20 s).
- **The store is mutated where it changed.** It has about 4,800 mutants, and a
  viable one rebuilds and relinks the store's 137 test binaries and reruns
  them. A local sample of 41 mutants (`cargo mutants --package maidan-store
  --test-tool nextest --sharding round-robin --shard 0/120`, 16 cores,
  2026-09-30) took 164 minutes: 32 viable, all caught, at 5 minutes each on
  average and 14 at most, and 9 unviable at half a minute. That is 4 minutes a
  mutant (3 for the first 20, before other builds loaded the machine). At
  three times that on a four-core runner a whole sweep is about 960 runner
  hours, so it does not run. Instead `scripts/mutants.sh plan` counts the
  mutants in the store code changed since the last commit more than 25 hours
  old (the nights overlap by an hour rather than leave a gap) and splits them
  into shards of 20, which at the sampled rate take about four hours on a
  runner (three at the unloaded rate), inside the step's 300 minutes.
- **At most 10 store shards run**, assuming GitHub's 20 concurrent jobs for a
  public repository on the free plan: with the eight other mutation jobs, the
  benchmark and the fuzz job, the night fits the pool (the planning job ends
  before the shards start). That is 200 mutants a night; in the week to
  2026-09-30 one day's store changes produced up to about 390, so a busy day
  overflows. The planning job then warns and names the manual run
  (`store_base`, `store_first_shard`) that covers the rest; run it before
  `main` moves, since the shards are cut from the diff to the head.
- **A shard that does not finish fails.** Each cargo-mutants step has its own
  time limit, shorter than the job's, so a shard cut off by it still reports:
  `scripts/mutants.sh report` writes the counts and the missed and timed-out
  mutants to the job summary, uploads `mutants.out/`, and fails the job if the
  run was cut short, the unmutated tree failed its tests, or cargo-mutants
  failed. There is no `continue-on-error`: until this was fixed every shard
  failed in its first seconds (an `--output` directory that did not exist) and
  the night reported success.
- **Remeasure before resizing.** Run a round-robin shard of the store as above
  and read the phase durations in `mutants.out/outcomes.json`; change
  `STORE_MUTANTS_PER_SHARD` or `STORE_MAX_SHARDS` in `scripts/mutants.sh` and
  this section together.
- **Exclusions** live in `.cargo/mutants.toml`, each with its reason. The one
  there is the store's test harness (`test_support.rs`): mutating the Docker
  check to "no daemon" turns every Postgres test into a skip, so that mutant
  survives by construction. Code is excluded only when mutating it cannot say
  anything about the code the tests check; hard to test is not a reason.
