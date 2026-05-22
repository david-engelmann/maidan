# Cluster A retro — Foundation

> Closing wave for Cluster A · target tag `v0.0.1`.

The first cluster turned the empty repo into a working substrate. Six
PRs landed across two working weeks; every PR after this one starts in
a workspace that already builds, tests, and deploys.

## What shipped

- **PR #7** — `chore: governance + workspace scaffold` — MIT LICENSE,
  CONTRIBUTING, SECURITY, CHANGELOG, .gitignore, .editorconfig,
  toolchain pin, rustfmt + clippy + cargo-deny configs, the Cargo
  workspace with 13 crate stubs, the Makefile, and the Obsidian docs
  vault under [[docs/README|docs/]].
- **PR #8** — `feat(maidan-store): postgres impl + schema 0001` —
  schema 0001 (workspaces, members, channels, threads, messages,
  mentions, votes, references, artifacts, audit), full Postgres CRUD,
  idempotent migration runner, testcontainers integration test.
- **PR #9** — `chore(infra): docker + k8s deployment manifests` —
  prod-grade multi-stage Dockerfile (cargo-chef + distroless),
  `Dockerfile.dev` (cargo-watch), `docker/Dockerfile.db` (pgvector +
  bundled schema), `compose.yaml` + `compose.dev.yaml`, full Kustomize
  `k8s/` (base + dev + prod overlays).
- **PR #10** — `feat(maidan-artifacts): LocalFsStore + content-
  addressing` — `Sha256` newtype, `ArtifactStore` trait, `LocalFsStore`
  with sha-fanout + atomic writes + dedup. 13 tests including a
  50-task concurrent-put stress test.
- **PR #11** — `feat(maidan-server): /health endpoint + axum app` —
  split into lib + bin, env-driven `Config`, `AppState` over
  `Arc<dyn Trait>`, `/health` reports DB + storage with structured
  body, real-process e2e test.
- **PR #12** — `feat(maidan-store): sqlite parity` — SQLite dialect
  of schema 0001, `SqliteStore` mirroring the Postgres modules,
  `Dialect::from_url` routing, shared `tests/common/mod.rs` so both
  backends run the same assertion suite, cross-dialect parity test.

## What was deferred

| To           | What                                                            | Why                                                                    |
|--------------|-----------------------------------------------------------------|------------------------------------------------------------------------|
| GitHub Actions PR | Lint / secrets / test / integration / e2e CI workflows     | Wanted local-green before paying CI iteration cost; lands next.        |
| Cluster B    | Routing + event bus + MCP surface                                | The substrate is in place; behavior comes next.                        |
| Cluster E    | S3-compatible artifact backend; streaming put/get                | LocalFs covers single-node + dev; multi-node deferral is intentional.  |
| Cluster T    | Observability — OTLP, metrics, structured JSON logs              | `/health` is the floor; full telemetry is a cross-cutting track.       |
| Cluster T    | SQLite WAL mode + busy_timeout pragma tuning                     | Default journal mode is fine for dev; tuning is a Cluster T concern.   |
| Cluster V    | Schema parity property test diffing information_schema rows      | Shared `common` suite + roundtrip + parity test is enough for v0.0.1.  |
| Cluster H    | Graceful shutdown, request-id middleware, web UI                 | Cluster H scope.                                                       |
| Cluster A retro candidate | Coverage gate via cargo-llvm-cov                    | Tooling lands with CI workflows.                                       |

## Surprises

- Two toolchain bumps were forced by transitive deps, not our code:
  1.82 → 1.85 (getrandom 0.4 needed edition 2024), then 1.85 → 1.88
  (icu_* and idna needed rustc 1.86+).
- `.gitignore`'s `*.db` rule silently matched `docker/Dockerfile.db`
  on the deploy PR. Caught only by reading `git status` carefully —
  added an explicit exception.
- serde's externally-tagged `enum SubsystemStatus { Ok, Error(String) }`
  serializes exactly as needed (`"ok"` / `{"error": "..."}`) with no
  attributes, so the `/health` body shape stayed minimal.
- SQLite's UNIQUE-violation path returns `sqlx::Error::Database` with
  `is_unique_violation() == true` — identical to Postgres — so the
  Conflict mapping mirrors Postgres byte-for-byte. No special-casing.

## Decisions

- **`Arc<dyn Trait>` in `AppState`, not concrete backends.** Pays off
  immediately when integration tests build the router with a tempdir
  artifact store. Stays.
- **`Dialect::from_url` for runtime routing**, not a single sqlx-Any
  pool. sqlx-Any doesn't cover the features we use; explicit branching
  with a tiny enum is clearer and lossless. Stays.
- **Shared `tests/common/mod.rs` for store roundtrip assertions.** Tests
  duplicate plumbing (testcontainer vs in-memory) but share the
  assertion body. Future backends get the full suite for free.
- **No GitHub Actions yet.** Local-first CI proved sufficient for the
  scaffold; landing CI before there's a real binary to exercise would
  have been busy work. Lands in the next PR.
- **Docker compose split into `compose.yaml` and `compose.dev.yaml`**
  rather than one file with profile flags. Two files are simpler to
  reason about; one is read by hand, the other by `docker compose -f`.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Persistent core schema (Postgres + SQLite)              | `v0.0.1`           |
| Content-addressed artifact body store (LocalFs)         | `v0.0.1`           |
| `/health` endpoint reporting DB + storage status        | `v0.0.1`           |
| `docker compose up` brings up Postgres + MinIO + server | `v0.0.1`           |
| `docker compose -f compose.dev.yaml up` for hot reload  | `v0.0.1`           |
| Kustomize base + dev + prod overlays                    | `v0.0.1`           |
| testcontainers-backed integration suite                 | `v0.0.1`           |
| Dialect detection from `DATABASE_URL` prefix            | `v0.0.1`           |
| Cross-dialect parity test                               | `v0.0.1`           |

See [[Capabilities]].

## Risks identified + mitigated

- **Concurrent artifact puts could leak tempfiles.** Mitigated by
  per-task UUID tempfile + rename-as-commit. Verified by a 50-task
  stress test that asserts both "exactly one body file" and "zero
  leaked .tmp- files".
- **Migrations run twice.** Mitigated by tracking applied versions in
  `maidan_migrations`. Tested for both dialects via the
  `migrations_are_idempotent` test.
- **Toolchain drift between local and CI.** Mitigated by pinning the
  toolchain in `rust-toolchain.toml` (currently 1.88); rustup auto-
  installs on first build.
- **Secret leakage in compose / k8s manifests.** Mitigated by
  `secret.example.yaml` template that documents the keys without
  values, and a `.gitignore` rule that blocks `.env`, `*.pem`, `*.key`,
  `maidan.toml` from accidental commit. The dev compose stack uses
  hard-coded local-only credentials and is not intended for any other
  context.

## Risks identified + still open

- **No `cargo deny` or `trufflehog` in CI yet.** Local-only is the
  current discipline; once CI lands they become required-status-checks
  on `main`.
- **No mutation tests, no coverage gate.** Tooling is named in
  [[Conventions]] but not wired. Lands in the CI workflows PR.
- **SQLite default journal mode is `DELETE`, not `WAL`.** Fine for
  dev; production deployments using SQLite (if any) should pin WAL.
  Tracked for Cluster T.
- **`docker/Dockerfile.db` bundles schema 0001 in
  `/docker-entrypoint-initdb.d` for fresh-volume bootstrap.** The
  server also applies migrations on boot, so this is a redundancy —
  but it means schema drift between the image and the migration file
  is possible. Should be reconciled in Cluster T (single source of
  truth: the migration file, image stops bundling).

## Forward look

Cluster B picks up next: routing, event bus, MCP surface. The first
real behavior on top of the substrate Cluster A delivered. Highest
priorities for the cluster kickoff:

1. GitHub Actions CI workflows (cleanup PR before B starts proper).
2. Channel + thread routing service (`maidan-router`).
3. Postgres `LISTEN/NOTIFY` wired through `maidan-bus`.
4. MCP server surface exposing the workspace as tools.

See [[Roadmap]] for the full ladder.

## Acknowledgements

PRs reviewed and merged by the maintainer (David Engelmann). Cluster A
was a solo cluster; future clusters will accrue external contributors.
