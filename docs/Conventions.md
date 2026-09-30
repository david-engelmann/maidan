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

The retrospective is mandatory. The repository squashes with the branch's commit
messages as the body, not the PR description, so the retro lives in the PR;
anything a later reader needs from it (a deferral, a decision) also goes into
[`docs/Open Work.md`](Open%20Work.md) or [`docs/Decisions.md`](Decisions.md) in
the same PR.

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
| `a2a tck` | the official A2A conformance kit | no |
| `mcp inspector (report-only)` | the official MCP Inspector against the server | no |
| `sdk interop (report-only)` | the four SDKs against a live server | no |
| `pitr drill` | point-in-time recovery to a chosen moment | no |
| `sqlite backup drill` | a SQLite snapshot taken mid-write, restored over a killed server's files | no |
| `replica routing (LSN)` | read-your-writes across a streaming replica | no |
| `ui tests (playwright)` | the `/ui` specs in a headless browser | no |

The docs site builds in [`docs.yml`](../.github/workflows/docs.yml) (`mdbook`,
not required). `nightly.yml` runs the slower checks: cargo-mutants over the
store and artifacts, a benchmark build, and five minutes of fuzzing per target
in `fuzz/` (the egress SSRF guard, room URIs, waiter results, event
type ids, content keys). A new parser of untrusted input gets a target there.
