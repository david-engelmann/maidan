# Cluster 7.0 — Bus pointer delivery

Cluster 6.0 closed delivery observability at **`v6.0.0`**. Operators can alert on
subscribe lag, replay outcomes, indexer age, and Postgres listener health, but
the Postgres bus still ships the **full JSON envelope** on `NOTIFY` (7990-byte cap)
even though every mutation already appends to `maidan_events` with a stable
`log_id` ([`routes::publish`](../../crates/maidan-server/src/routes.rs)).

[[Decisions]] recorded that pointer-style NOTIFY should be revisited
once the persistent event log exists (Cluster D). This cluster implements that
revisit without promising at-most-once delivery.

> **Goal:** On Postgres, `pg_notify` carries a minimal pointer (`log_id`, optional
> `workspace_id`); the listener hydrates [`BusEnvelope`](../../crates/maidan-types)
> from `maidan_events`. Large events no longer fail publish solely due to NOTIFY
> payload size. `InMemoryBus` stays full-envelope for dev parity.
>
> **Target tag:** `v7.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 7.0.1–4   | `feat: Cluster 7.0 bus pointer delivery` (#162)                        | —     |
| 7.0.kickoff | `docs: Cluster 7.0 kickoff plan` (#161)                                | —     |
| 7.0.retro | `docs(retro): Cluster 7.0 retrospective + v7.0.0 tag prep` (this PR)   | —     |

## Order

1. **7.0.1** — add `Store::get_stored_event(log_id)` (Postgres + SQLite); shared
   assertions in `tests/common` or crate integration tests. Return
   `StoreError::NotFound` when missing.
2. **7.0.2** — `PostgresBus::publish` serializes a small notify body (e.g.
   `{"log_id":N,"workspace_id":"..."}`) instead of the full envelope; cap check
   applies to the pointer JSON only. Background `LISTEN` task holds `PgPool` (or
   store handle) and loads the row by `log_id`, builds `BusEnvelope`, fans into the
   process-local broadcast. Malformed notify payloads are dropped with `tracing::warn`.
3. **7.0.3** — integration tests in `maidan-bus` (testcontainers): round-trip;
   event whose envelope JSON exceeds `PAYLOAD_LIMIT` still delivers after append.
   Keep `InMemoryBus` unchanged (no pointer path).
4. **7.0.4** — update [[Decisions]] (“Postgres NOTIFY full-payload” entry: pointer
   is default on Postgres); [[Architecture]] bus diagram; [[Production]] note on
   size limits; optional counter `maidan_bus_notify_hydrate_total{result}` if cheap.
5. **7.0.retro** + `v7.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + coverage floor from 5.0).
- Postgres `publish` + `subscribe` deliver events via log-id pointer + store hydrate.
- Events larger than the legacy NOTIFY cap publish successfully on Postgres (row in
  `maidan_events` is authoritative).
- [[Decisions]] and [[Architecture]] describe pointer-default semantics.
- [[Retros/README]] includes Cluster 7.0; `v7.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Extra DB read per NOTIFY | Single `get_stored_event` by PK; acceptable vs 8KB JSON on wire. |
| NOTIFY still at-most-once | Document unchanged standing risk; replay/auto-replay paths remain. |
| Race: row not visible yet | Publish only NOTIFY after `append_event` commits in same process order. |
| Federation / alternate publish paths | Audit all `bus.publish` call sites; only HTTP `publish` helper needs audit if others bypass log. |
| Hydrate failure on missing row | Drop + warn + metric; subscriber may still recover via event-log replay (6.0 metrics). |

## Out of scope

- Outbox pattern, transactional publish, or guaranteed at-least-once semantics.
- Changing `InMemoryBus` to pointer semantics (dev/test backend stays in-process JSON).
- Coverage floor bump to 11%+ (separate cluster).
- Per-model embedding tables / SQLite semantic search.
- SSE for MCP `resources/subscribe` (Cluster B deferral).

## Dependencies

- **7.0.1** before **7.0.2** (bus hydrate needs store API).
- **7.0.2** before **7.0.3** (tests exercise new path).
- **7.0.4** after **7.0.2** (docs describe shipped behavior).

## Alternative next cluster (not this wave)

**Coverage depth (`v7.0.0` avoided):** measured bump toward 11%+ — deferred from
Cluster 5/6 when operator-facing reliability work ranked higher.

## References

- Event log + publish: `maidan-server/src/routes.rs` (`append_event` then `bus.publish`).
- Current Postgres bus: `maidan-bus/src/postgres.rs` (`PAYLOAD_LIMIT`, full JSON).
- NOTIFY decision: [[Decisions]] (“Postgres NOTIFY full-payload over event-id-pointer”).
- Subscriber recovery: [[Retros/Cluster 4.0]], [[Retros/Cluster 6.0]] metrics.
- Replay from log: `maidan-server/src/event_stream.rs`.
