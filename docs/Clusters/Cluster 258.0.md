# Cluster 258.0 — event-bus self-healing NOTIFY floor

> **Program D (scale & durability), part 1.** Phase XXIV post-gate hardening.
> Tag **`v258.0.0`**. No new gate tag.

## Goal

Close the one gap in the Postgres `LISTEN`/`NOTIFY` bus's optimistic delivery path:
when the listener connection drops (or a NOTIFY is silently lost across a
transparent reconnect), the events appended during the gap never reach the local
broadcast — subscribers on that replica miss them. The floor tracks a high-water
`log_id` and back-fills the missed range from the event log, so the live path is
self-healing.

## Scope

| Change | Where |
|--------|-------|
| `list_after_global(after_id, limit)` + `max_event_id()` (cross-workspace log reads) | `maidan-store/src/postgres/events.rs` |
| High-water tracking + gap back-fill + reconnect catch-up in the LISTEN task; `drain_new_events`; `pub backfill()` heal hook | `maidan-bus/src/postgres.rs` |
| `HydrateResult::Backfilled` + snapshot field | `maidan-bus/src/hydrate_stats.rs` |
| `maidan_bus_notify_hydrate_total{result="backfilled"}` | `maidan-server/src/metrics.rs` |

## Design decisions

- **The NOTIFY becomes a wake-up, the log is the source of truth.** The listener
  seeds a high-water mark from `MAX(id)` at startup (so it never replays history),
  and on each pointer advances it. A pointer whose id exceeds `high_water + 1`
  signals a silently-missed range — the listener back-fills `(high_water, id)` from
  the log before delivering the pointer's own event.
- **Always hydrate the pointer's own id — never skip on `<= high_water`.** A lower
  id can commit *after* a higher one (concurrent transactions), so its NOTIFY may
  arrive when the high-water is already past it; skipping would drop it. Hydrating
  it unconditionally (and letting the occasional duplicate through — the local
  broadcast and at-least-once path both tolerate re-delivery) is the safe choice.
- **Reconnect catch-up.** On a listener error, after the retry sleep, the listener
  drains everything above the high-water to head — healing the events appended while
  it was disconnected, which is the primary gap. This runs over the (independent)
  pool, so it works even while the LISTEN socket is down.
- **Batched, best-effort.** The drain pages in `BACKFILL_BATCH` chunks (a long
  disconnect heals without loading the whole gap at once) and stops on a store error
  to retry on the next NOTIFY. It is not a durability guarantee — the transactional
  outbox + at-least-once cursor remain the durable path; this only makes the
  *optimistic* local broadcast self-healing.
- **Observability.** Back-filled events count as `HydrateResult::Backfilled` (a
  distinct label from live `ok` hydrations), so an operator can see the floor firing.

## Non-goals / deferred

- **Batched `pg_notify` on the publish side** — still declined (the hot path has no
  natural batch; this LISTEN-side floor is the complementary, enabling piece).
- Other Program D items: read-replica routing, backup/DR runbook, chaos harness.

## Risks

- Delivery-critical code. Covered by the both-path bus test (`backfill_drains_the_
  missed_range_onto_the_broadcast` + the existing round-trip and NotFound tests, all
  green) and the always-hydrate invariant that preserves the pre-existing per-pointer
  behaviour.
