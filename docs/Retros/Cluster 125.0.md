# Cluster 125.0 retro — At-least-once event delivery

> Tag **`v125.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. The
> first multi-PR cluster of the phase (foundation → behavior change → docs).

## What shipped

- **Store foundation** (125.0.1, #341): `maidan_events.inserted_at` (the DB
  insert wall-clock, set by the app at append — distinct from the caller's
  `occurred_at`) + `Store::list_events_after_stable(ws, after_id, stable_before,
  limit)`, the stability-gated gap-safe read. Additive; Postgres-verified in CI.
- **At-least-once delivery** (125.0.2, #342): `event_stream::reconcile_deliver`
  + the opt-in `at_least_once` subscribe flag (requires `workspace_id +
  consumer_id`). For those subscriptions, delivery is cursor-driven: poll
  stable rows from the durable cursor in `log_id` order, deliver, advance the
  cursor; the bus NOTIFY is a wake hint. `ws.rs` branches reconcile vs the
  unchanged optimistic `forward_bus_items`. Window + cadence live on `AppState`
  (env-sourced once).
- **Docs** (125.0.3): the ADR (cursor reconciliation + time-based stability
  horizon), Production.md env vars, and the "At-least-once delivery" contract.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Follow-on | MCP SSE (`/mcp/stream`) `at_least_once` parity | This cluster wired WebSocket; MCP reuses the same `reconcile_deliver`. |
| Documented caveat | Strictness vs a write txn longer than the window | The time-horizon assumes bounded transaction duration; size `W` above the slowest writer. |
| Out of scope | A commit-sequence column / logical decoding | The strict-but-invasive alternative; rejected in the ADR. |

## Surprises

- **The premise was half-wrong — in our favor.** The chosen task was "dedup /
  idempotency", but mapping the path showed dedup was already handled (the
  per-stream watermark drops re-published/NOTIFY-duplicated ids, and the cursor
  floors reconnects). The real, open hole was the *opposite*: silent **gaps**
  from out-of-order publishes. Surfacing that reframed the cluster before any
  code was written.
- **A high-water mark can't both go fast and never gap.** A single monotonic
  watermark conflates "delivered up to N" with "delivered N"; it can't backfill
  a sub-N late commit without either duplicates or gaps. The resolution was to
  give the gap-free path its own contiguous, stability-gated cursor and accept a
  latency floor — chosen as an explicit, opt-in trade.
- **`occurred_at` is not insert time.** It's caller-supplied and skewable, so it
  could not gate the horizon — hence the new `inserted_at` column.
- **The flaky test taught the contract.** The e2e first flaked because it
  reconnected before the cursor's async commit — which is *exactly* the
  at-least-once re-delivery the design permits. The fix (wait for the cursor)
  made the test assert the no-re-delivery optimization deterministically.

## Decisions

- **Opt-in, not default.** `at_least_once` is a subscribe flag; the optimistic
  low-latency path is untouched for everyone else. Zero-regression migration.
- **Time-based stability horizon** over a commit-sequence column — additive,
  identical on both backends, with a documented latency/assumption trade.
- **Foundation merged before the behavior change.** 125.0.1 (additive) landed
  and was Postgres-verified before the riskier consumer-loop PR.

## Capability table extension

| Capability | Where |
|------------|-------|
| Opt-in at-least-once subscribe (gap-free, in-order, per-consumer) | `at_least_once` flag, `event_stream::reconcile_deliver`, `Store::list_events_after_stable` |
| Insert-time stability column | `maidan_events.inserted_at` (migrations 0031 / 0030) |

## Risks identified + still open

- **Window vs latency.** `MAIDAN_DELIVERY_STABILITY_SECS` (default 2s) is the
  fresh-event latency floor *and* the long-transaction safety margin — one knob,
  two concerns. Operators must size it to their slowest write transaction.
- **WebSocket only.** MCP SSE subscribers can't yet opt in (follow-on).

## Forward look

The delivery story is now: optimistic low-latency by default, opt-in gap-free
at-least-once when a consumer needs it. Natural next: MCP SSE parity, or revisit
the standing "exactly-once bus" risk (now largely addressed at the consumer).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
