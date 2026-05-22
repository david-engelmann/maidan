# Cluster A — Foundation

The first cluster. Turns the empty repo into a working substrate so
every subsequent cluster has a workspace that builds, tests, and
deploys.

> **Goal:** a fresh clone runs `docker compose up`, exposes `/health`,
> persists data in Postgres (and SQLite), and survives CI.
>
> **Target tag:** `v0.0.1`.

## PRs

| #  | Title                                                       | Issue |
|----|-------------------------------------------------------------|-------|
| 1  | `chore: governance + workspace scaffold`                    | #1    |
| 2  | `feat(maidan-store): postgres impl + schema 0001`           | #2    |
| 3  | `feat(maidan-artifacts): LocalFsStore + content-addressing` | #3    |
| 4  | `feat(maidan-server): /health endpoint + compose.yaml`      | #4    |
| 5  | `feat(maidan-store): sqlite parity`                         | #5    |
| 6  | `docs(retro): Cluster A retrospective + v0.0.1 tag prep`    | #6    |

## Exit criteria

- `git clone && docker compose up && curl localhost:8080/health`
  returns 200 on a fresh machine.
- CI green on `main`.
- testcontainers integration suite passes locally and in CI.
- Workspace coverage ≥ 60%.
- [[Retros/README|Cluster A retro]] merged.
- `v0.0.1` tagged and signed.

## Risks

| Risk                                                            | Mitigation                                                              |
|-----------------------------------------------------------------|-------------------------------------------------------------------------|
| testcontainers slow in CI                                       | Cache the `postgres:16+pgvector` image.                                 |
| sqlx compile-time query check requires a live DB                | Use `SQLX_OFFLINE=true` with checked-in `.sqlx/` cache.                 |
| Coverage tooling flaky on first install                         | Pin `cargo-llvm-cov` version in CI.                                     |
| Branch protection blocks first PR landing CI                    | Add required-status-checks *after* CI lands, not as a precondition.     |
| Dialect parity tests double the integration runtime             | Acceptable cost; parallelize testcontainers across cores.               |
