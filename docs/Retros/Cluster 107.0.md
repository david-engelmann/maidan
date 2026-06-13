# Cluster 107.0 retro — Pool & timeouts configurable

> Tag **`v107.0.0`**. Second cluster of Phase XX (hot-path hardening).

## What shipped

- **`config::DbConfig`** — `MAIDAN_DB_MAX_CONNECTIONS`, `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS`, `MAIDAN_DB_STATEMENT_TIMEOUT_MS`, `MAIDAN_DB_BUSY_TIMEOUT_MS`, parsed by a pure `from_lookup` fn with unit tests (defaults / overrides / non-numeric rejection). Defaults reproduce prior behavior. (107.0.1+.2, #297)
- **Pool wiring** — both pools read `max_connections` (`None` → dialect default 16/8) and `acquire_timeout`; Postgres applies `statement_timeout` per connection via `after_connect` only when configured; SQLite `busy_timeout` via the new non-breaking `configure_sqlite_pool_with`. (107.0.1+.2, #297)
- **Migration exemption** — the boot-migration session resets `statement_timeout = 0` before `pg_advisory_lock`, so a configured cap can't kill the cross-replica lock *wait* (Cluster 105). Verified against real Postgres by `concurrent_migrations`. (107.0.1+.2, #297)
- **Docs** — `docs/Production.md` `MAIDAN_DB_*` table + a "Database tuning" section (the `replicas × max_connections` caveat; the `statement_timeout` ⇄ reindex/migration interaction). (107.0.3, this PR)

## What was deferred / not covered

- **Read replicas / external pooler (PgBouncer)** — [[Open Work]], not this cluster.
- **Per-route / per-query timeout overrides** — one global `statement_timeout` with documented exemptions is enough for the gate.
- **In-server operator reindex under a configured `statement_timeout`** stays subject to the cap (documented); large reindexes should use the `maidan reindex-embeddings` CLI, which connects its own uncapped pool. Active per-operation exemption would require resetting the cap on the search path — out of scope.

## Surprises

- **The advisory lock and `statement_timeout` collide.** Setting `statement_timeout` via `after_connect` applies to *every* pooled connection — including the one a booting replica uses to wait on `pg_advisory_lock` while another replica migrates. A low cap would kill that legitimate wait. The migration session now resets the timeout to 0 on its own connection; this is unconditional (a no-op when no cap is set), so the two clusters compose cleanly.
- **Dialect-specific defaults vs one env var.** "Reproduce current behavior" meant Postgres 16 / SQLite 8 — so `max_connections` is `Option<u32>` (`None` keeps the per-dialect default) rather than a single hardcoded fallback.

## Decisions

- **Env-driven pool config with behavior-preserving defaults**, parsed by a pure `from_lookup` so the logic is unit-testable without touching `std::env` (no parallel-test env races). `statement_timeout` defaults to *disabled* so the default is exactly prior behavior; operators opt in. See `docs/Decisions.md` (advisory-lock ADR) and `docs/Production.md`.

## Capability table extension

| Capability | Where |
|------------|-------|
| Env-tunable DB pool size + acquire timeout | `config::DbConfig`, `main.rs` pool construction |
| Postgres `statement_timeout` / SQLite `busy_timeout` | `after_connect` cap (migration-exempt), `configure_sqlite_pool_with` |

## Risks

- **Total connections = replicas × `MAIDAN_DB_MAX_CONNECTIONS`** must stay under Postgres `max_connections`; documented in Production with an example.
- A too-aggressive `statement_timeout` can interrupt legitimate slow queries (notably the in-server reindex); the default-disabled posture and the docs caveat mitigate this.

## Next

Cluster **108** — adaptive outbox relay (react to NOTIFY / back off when idle instead of a fixed 50 ms poll).
