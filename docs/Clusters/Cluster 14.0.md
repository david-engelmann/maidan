# Cluster 14.0 — SQLite transactional outbox

Cluster 13.0 (delivery ledger) ships at **`v13.0.0`**. Cluster 10.0 introduced the Postgres
transactional outbox at **`v10.0.0`**; Cluster 12.0 added quarantine at **`v12.0.0`**. SQLite
deployments still use append-then-publish with an in-memory bus — the crash window between
`maidan_events` commit and `bus.publish` remains on the default dev/single-node SQLite path.

> **Epic pick:** **SQLite transactional outbox** (deferred from Cluster 10.0 and 12.0 retro).
>
> **Goal:** On SQLite, enqueue `maidan_outbox` in the same transaction as `append_event`;
> run the existing relay against `InMemoryBus` after commit; reuse quarantine semantics and
> metrics. Document parity with Postgres outbox behavior.
>
> **Target tag:** `v14.0.0`.

## Alternatives considered (not this cluster)

| Epic | Why deferred |
|------|----------------|
| SQLite semantic (`sqlite-vec`) | Extension maturity; sqlx integration risk. |
| MCP `resources/subscribe` SSE | Long-standing deferral; separate protocol surface. |
| S3 multipart uploads | Cluster E follow-up; less coupled to delivery track. |

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 14.0 kickoff plan`                                      | TBD   |
| 14.0.1     | `feat(maidan-store): SQLite outbox schema + transactional append`      | TBD   |
| 14.0.2     | `feat(maidan-server): outbox relay on SQLite + metrics`                | TBD   |
| 14.0.3     | `test: SQLite outbox + relay integration`                              | TBD   |
| 14.0.4     | `docs: SQLite outbox in Decisions/Architecture/Production`           | TBD   |
| 14.0.retro | `docs(retro): Cluster 14.0 retrospective + v14.0.0 tag prep`            | TBD   |

## Order

### 14.0.1 — store + migration

- Migration **`migrations/sqlite/0013_outbox.sql`** (version 13 in `migrate.rs`):
  - `maidan_outbox` with `quarantined_at` from day one (matches Postgres v13+v14).
  - Partial index `idx_outbox_relayable` on relayable rows.
- **`maidan-store/src/sqlite/outbox.rs`** — mirror Postgres helpers:
  `list_pending`, `mark_published`, `record_attempt`, `quarantine`, counts, age gauge input.
- **`sqlite/events.rs`** — `append` in a transaction; `outbox::enqueue_in_tx`.
- **`maidan-store/src/outbox.rs`** — `OutboxBackend` enum (`Postgres` | `Sqlite`) for relay/metrics.

### 14.0.2 — server

- **`OutboxRelay`** — take `OutboxBackend` instead of `PgPool` only; load events from the
  correct dialect module.
- **`main.rs`** — enable `outbox_relay` on SQLite; spawn relay with `InMemoryBus`.
- **`AppState`** — `outbox_backend: Option<OutboxBackend>` (replace `outbox_pool`).
- **`metrics.rs`** — refresh outbox gauges on SQLite when relay enabled.

### 14.0.3 — tests

- **`maidan-store/tests/outbox_sqlite.rs`** — parity with Postgres outbox tests (memory DB).
- **`maidan-server`** — SQLite harness: HTTP mutation defers bus until relay runs (mirror
  `outbox_http_e2e` pattern without testcontainers).

### 14.0.4 — docs

- [[Decisions]] — extend outbox ADR: SQLite parity; InMemoryBus relay path.
- [[Architecture]] — SQLite TX → outbox → relay → in-memory fan-out.
- [[Production]] — note SQLite uses same `MAIDAN_OUTBOX_MAX_ATTEMPTS` env.

### 14.0.retro

- Retro, CHANGELOG `v14.0.0`, [[Capabilities]], tag.

## Exit criteria

- CI green on `main`.
- SQLite `append_event` + outbox row commit atomically; relay publishes to `InMemoryBus`.
- Quarantine + metrics behave like Postgres (same env vars).
- `v14.0.0` tagged after retro.

## Risks

| Risk | Mitigation |
|------|------------|
| Dual outbox code paths drift | Shared `OutboxBackend`; store tests on both dialects |
| InMemoryBus ≠ NOTIFY semantics | Document: ordering fix only; multi-process still needs Postgres |

## Out of scope

- Postgres NOTIFY on SQLite.
- SQLite semantic search.
- MCP `resources/subscribe` streaming.
- S3 multipart.

## Follow-on

| Cluster | Tag | Theme |
|---------|-----|--------|
| **15.0** | `v15.0.0` | Epic pick at 14 retro (semantic SQLite, MCP subscribe, S3 multipart) |

## References

- Cluster 10.0 / 12.0 outbox: [[Clusters/Cluster 10.0]], [[Retros/Cluster 12.0]].
- `maidan-server/src/outbox_relay.rs`, `maidan-store/src/postgres/outbox.rs`.
