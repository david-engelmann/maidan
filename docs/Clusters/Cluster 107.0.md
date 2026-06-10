# Cluster 107.0 — Pool & timeouts configurable

**Theme:** Make the database connection pool and query timeouts configurable, with safe documented defaults.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XX · tag **`v107.0.0`**.

**Predecessor:** [[Clusters/Cluster 101.0]].

---

## Problem

The pool is **hardcoded** in `crates/maidan-server/src/main.rs`: `PgPoolOptions::new().max_connections(16)` (line 59) for Postgres and `.max_connections(8)` (line 91) for SQLite, with **no `acquire_timeout`** and **no statement-level timeout**. Under the multi-replica, bursty load this ladder enables (and behind the N+1 hot paths until [[Clusters/Cluster 106.0]] lands), 16 connections starve quickly, a connection acquire blocks on the sqlx default (30 s), and a single runaway query has no server-side cap.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Config** | Env-driven `MAIDAN_DB_MAX_CONNECTIONS`, `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS`, `MAIDAN_DB_STATEMENT_TIMEOUT_MS` (and SQLite `busy_timeout`), parsed in `config.rs` with documented defaults that preserve today's behavior. |
| **Server** | `PgPoolOptions` / `SqlitePoolOptions` read the config; set `acquire_timeout`; Postgres sets `statement_timeout` per connection via `after_connect`; SQLite sets `busy_timeout`. |
| **Tests** | Config parse + default-value tests; optionally a saturation test proving `acquire_timeout` surfaces a clean 503/Problem rather than hanging. |
| **Docs** | [[Production]] DB-tuning env table; note the interaction with replica count (total connections = replicas × `MAIDAN_DB_MAX_CONNECTIONS`). |

## Non-goals

- Read replicas / connection pooler (PgBouncer) — [[Open Work]].
- Per-query / per-route timeout overrides — one global statement timeout with documented exemptions is enough for the gate.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 107.0.1 | `feat(server): configurable pool size + acquire timeout (pg + sqlite)` |
| 107.0.2 | `feat(server): statement_timeout / busy_timeout via after_connect` |
| 107.0.3 | `docs(production): database tuning env vars` |
| 107.0.retro | `docs(retro): Cluster 107.0 + v107.0.0 tag prep` |

## Exit criteria

- Pool size, acquire timeout, and statement/busy timeout are env-configurable; **defaults reproduce current behavior**.
- Connection-acquire exhaustion returns a clean error (no indefinite hang).
- `MAIDAN_DB_*` documented in [[Production]] with the replica-multiplier caveat.
- `v107.0.0` tagged after retro.

## Ordering & risks

- **Independent**; pairs naturally with the multi-replica work (102–105) and with [[Clusters/Cluster 106.0]] (fewer queries → less pool pressure).
- **Risk — statement timeout kills legitimate long ops:** the reindex job ([[Clusters/Cluster 87.0]]) and migrations must be exempt or run on a connection without the cap. Set a generous default and document the exemption.
- **Risk — total connection blow-up under many replicas:** make the replica-multiplier explicit in docs so operators don't exceed Postgres `max_connections`.

## References

- [[Clusters/Product Ladder 102+]] Phase XX
- [[Clusters/Cluster 106.0]] (reduces pool pressure), [[Clusters/Cluster 87.0]] (reindex exemption)
- [[Production]], [[Operations]], [[Architecture]]
