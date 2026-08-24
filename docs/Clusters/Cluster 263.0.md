# Cluster 263.0 — consistency token on writes (read-replica, part 3)

> **Program D (scale & durability) — read-replica arc, part 3.** Phase XXIV
> post-gate hardening. Tag **`v263.0.0`**. No new gate tag.

## Goal

Emit the causality token. After a successful mutation, when a read replica is
configured, stamp the primary's WAL LSN on the response as `Maidan-Consistency-Token`
so a client can echo it on a later read (Cluster 264 routes on it).

## Scope

| Change | Where |
|--------|-------|
| `Store::write_lsn() -> Option<Lsn>` (Postgres `pg_current_wal_lsn()`; SQLite `None`) | `store.rs`, `postgres/mod.rs`, `sqlite/mod.rs` |
| `AppState.read_replica_enabled` flag (main.rs sets it from `MAIDAN_DB_REPLICA_URL`) | `state.rs`, `main.rs` |
| `consistency::middleware` — stamp the token on successful mutations | `consistency.rs`, `app.rs`, `lib.rs` |

## Design decisions

- **Capture the LSN after the handler — over-approximating is safe.** The middleware
  reads `pg_current_wal_lsn()` *after* the write commits, so a concurrent write can
  advance it in between; the token may be slightly *ahead* of this write's exact
  commit LSN, never behind. A read gated on a token that's `>=` the write is never
  stale — so over-approximation only makes a replica read wait a touch longer or fall
  back to the primary. Capturing inside each write tx (via `RETURNING
  pg_current_wal_lsn()`) would be exact but need per-method plumbing across both
  backends; the middleware is one clean chokepoint.
- **Gated on a configured replica.** The middleware only queries the LSN when
  `read_replica_enabled` (set from `MAIDAN_DB_REPLICA_URL`) — no replica means the
  token is unused, so we skip the extra round-trip entirely. Zero overhead and zero
  behaviour change for the default (no-replica) deployment.
- **`write_lsn` is a `Store` trait method, cross-backend.** Postgres returns
  `Some(pg_current_wal_lsn())`; SQLite returns `None` (no streaming replication), so
  the middleware simply emits no token on SQLite — no special-casing in the HTTP
  layer.
- **Only successful mutations.** `POST/PUT/DELETE/PATCH` with a 2xx status; reads and
  failures carry no token.

## Non-goals / deferred

- **Ingesting the token + routing reads** (Cluster 264) — this cluster only emits.
- Surfacing the token on MCP/WS responses — REST first; MCP/WS in a follow-up if
  needed (the arc's REST path is the primary orchestrator surface).

## Risks

- Inert without a replica (the default). Postgres-backed e2e proves the header is
  present on a mutation + parses as a `pg_lsn`, absent on reads, and absent when the
  replica flag is off.
