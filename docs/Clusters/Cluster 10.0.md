# Cluster 10.0 — Postgres transactional outbox

Cluster 9.0 raised coverage at **`v9.0.0`**. Clusters 6–8 made delivery
observable (metrics, pointers, hydrate counters), but the standing risk remains:
[`publish`](../../crates/maidan-server/src/routes.rs) appends to `maidan_events`
then calls `bus.publish` in a **separate** step — a crash between them leaves
subscribers without NOTIFY until replay, and NOTIFY itself is still fire-and-forget.

[[Decisions]] anticipated revisiting once the persistent event log exists (Cluster D).
This cluster adds a **minimal Postgres outbox** so the log row and a pending relay
row commit together; a background relay publishes pointers after commit.

> **Goal:** On Postgres, `append_event` + outbox enqueue in one transaction; a
> relay task drains pending rows and calls `PostgresBus::publish`. Document
> at-least-once relay semantics and subscriber idempotency expectations. SQLite /
> `InMemoryBus` keep the existing append-then-publish path.
>
> **Target tag:** `v10.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 10.0 kickoff plan`                                      | TBD   |
| 10.0.1     | `feat(maidan-store): outbox schema + enqueue in append transaction`      | TBD   |
| 10.0.2     | `feat(maidan-server): outbox relay worker (Postgres)`                   | TBD   |
| 10.0.3     | `test: outbox relay delivers after commit; crash-safe retry`             | TBD   |
| 10.0.4     | `docs: outbox semantics in Decisions/Architecture/Production`          | TBD   |
| 10.0.retro | `docs(retro): Cluster 10.0 retrospective + v10.0.0 tag prep`            | TBD   |

## Order

1. **10.0.1** — migration `maidan_outbox` (`id`, `log_id`, `created_at`,
   `published_at` nullable, `attempts`). Extend Postgres `append_event` (or
   `publish` helper) to insert outbox row in the **same transaction** as
   `maidan_events`. `Store` trait extension or Postgres-only hook documented.
2. **10.0.2** — `OutboxRelay` background task on Postgres deployments: poll pending
   rows (`published_at IS NULL`), `bus.publish` with pointer envelope, mark
   published on success; bounded backoff + metrics (`maidan_outbox_pending`,
   `maidan_outbox_relay_total{result}`) if cheap. Wire in `maidan-server` `main`
   alongside indexer; graceful shutdown drains or leaves rows for restart.
3. **10.0.3** — integration tests (testcontainers): append without calling
   `bus.publish` directly — relay delivers to subscriber; simulate relay retry
   (duplicate publish acceptable); ensure no NOTIFY before commit visible to other
   sessions.
4. **10.0.4** — update [[Decisions]] (publish ordering ADR); [[Architecture]]
   diagram (TX → outbox → relay → NOTIFY); [[Production]] ops (stuck pending rows).
5. **10.0.retro** + `v10.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + coverage floor from 9.0).
- Postgres HTTP `publish` path enqueues outbox in the same TX as `maidan_events`.
- Relay delivers pending rows to `PostgresBus` on green paths; metrics or logs
  prove relay activity.
- Docs state: relay is **at-least-once**; NOTIFY remains fire-and-forget;
  subscribers use `log_id` + replay for recovery (unchanged from 4–8).
- [[Retros/README]] includes Cluster 10.0; `v10.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Duplicate NOTIFY on relay retry | Document idempotency by `log_id`; pointer hydrate is row-keyed |
| Scope creep (exactly-once end-to-end) | Outbox only covers publish-after-commit; no consumer dedup table |
| SQLite / InMemory divergence | Postgres-only outbox; other backends unchanged |
| Federation publish paths | Audit all `publish` call sites; route through shared helper |
| Stuck pending rows | Metric/alert on age; relay backoff with cap |

## Out of scope

- NOTIFY / LISTEN guaranteed delivery (still fire-and-forget).
- Outbox on SQLite or `InMemoryBus`.
- Consumer-side deduplication store.
- Coverage floor bump to 11%+ (Cluster 9 follow-up).
- Per-model embedding tables / SQLite semantic search.

## Alternative next cluster (not this wave)

**Coverage floor 11%** — deferred from 9.0; CI-measured bump when headroom is
confirmed. Lower risk, less user-visible than outbox.

## References

- Current publish: `maidan-server/src/routes.rs` (`append_event` then `bus.publish`).
- Event log: `migrations/postgres/0006_event_log.sql`.
- Pointer NOTIFY: [[Retros/Cluster 7.0]], [[Retros/Cluster 8.0]].
- Publish ordering ADR: [[Decisions]] (“append then publish” entry).
