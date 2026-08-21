# Cluster 258.0 retro — a floor under the bus

> Tag **`v258.0.0`**. Phase XXIV (post-gate hardening). **Program D (scale &
> durability), part 1.** No new gate tag.

## What shipped

- The Postgres NOTIFY bus's LISTEN task now tracks a high-water `log_id` and
  **back-fills the missed range from the event log** on a detected gap (a pointer id
  above `high_water + 1`) or on a reconnect (drain to head after a listener error).
  New cross-workspace log reads `list_after_global` / `max_event_id`; a
  `Backfilled` hydrate stat + `maidan_bus_notify_hydrate_total{result="backfilled"}`
  metric; and a `pub backfill()` heal hook. The optimistic local-broadcast path is
  now self-healing across a `LISTEN` disconnect — Program D's opening cluster.

## Surprises / decisions

- **Always hydrate the pointer's own id — the design pivot the test forced.** My
  first cut treated a high pointer id purely as a "gap → drain the whole range to
  head" signal and did *not* separately hydrate the pointer. That broke the existing
  `pointer_notify_for_missing_log_id` test (a bogus high id recorded no `NotFound`)
  — and, more importantly, it was subtly wrong: a lower id that commits *after* a
  higher one (concurrent transactions) arrives as a NOTIFY when the high-water is
  already past it, and a drain-and-skip design would drop it. The fix: back-fill only
  the *exclusive* middle `(high_water, id)` on a gap, then **always** single-hydrate
  the pointer's own id, never skipping on `<= high_water`. Occasional duplicates are
  fine (the broadcast + at-least-once path tolerate re-delivery); a *drop* is not.
- **The NOTIFY is a wake-up, not the payload.** Reframing the pointer as "something
  changed, consult the log" (rather than "here is exactly event N") is what makes the
  floor possible — the authoritative set delivered is the log range, healed against
  the high-water, not whatever NOTIFYs happened to survive.
- **Reconnect catch-up runs over the pool, not the listener socket.** The drain uses
  the shared `PgPool`, independent of the `PgListener` connection, so it heals the
  disconnect gap even while the LISTEN socket is still down/reconnecting.
- **Boxing the classifier enum.** `NotifyOutcome::Envelope(BusEnvelope)` tripped
  `clippy::large_enum_variant` (288 vs 8 bytes) — boxed the large variant.
- **Scope honesty.** This is the *optimistic-path* floor, not a new durability
  guarantee; the transactional outbox (205–214) + at-least-once cursor stay the
  durable path. Batched `pg_notify` on the publish side stays declined.

## Capability table extension

| Change | Where |
|--------|-------|
| High-water gap back-fill + reconnect catch-up + `backfill()` | `maidan-bus/src/postgres.rs` |
| `list_after_global` / `max_event_id` | `maidan-store/src/postgres/events.rs` |
| `Backfilled` stat + metric label | `maidan-bus/src/hydrate_stats.rs`, `maidan-server/src/metrics.rs` |

## Risks identified + still open

- Delivery-critical; the always-hydrate invariant preserves the prior per-pointer
  behaviour, and the both-path bus test suite is green. A duplicate on the boundary
  between a back-fill and a live NOTIFY is possible and tolerated by design.

## Forward look

Program D continues: chaos/fault-injection harness (would exercise exactly this
floor — kill the DB mid-load, assert recovery), backup/restore + DR runbook, and the
larger read-replica routing refactor.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 257.0]].
