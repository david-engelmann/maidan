# Cluster 108.0 — Adaptive outbox relay

**Theme:** Make the outbox relay cadence adaptive — drain backlogs fast, idle cheaply — without changing delivery semantics.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XX · tag **`v108.0.0`**.

**Predecessor:** [[Clusters/Cluster 84.0]] (relay modes + configurable interval); outbox from [[Clusters/Cluster 10.0]] / [[Clusters/Cluster 12.0]].

---

## Problem

[[Clusters/Cluster 84.0]] already made the poll interval (`MAIDAN_OUTBOX_POLL_INTERVAL_MS`) and relay mode (`MAIDAN_OUTBOX_RELAY_MODE`) configurable. What remains is the **cadence shape**: `OutboxRelay::run()` (`crates/maidan-server/src/outbox_relay.rs:68-75`) drains exactly one `BATCH` (64, line 12) per tick and then **unconditionally `sleep(poll_interval)`** — regardless of whether more rows are pending or the queue is empty.

Consequences:
- **Backlog catch-up is slow:** N pending rows take ⌈N/64⌉ × interval. At 50 ms and a 1 000-row spike, that's ~0.8 s of avoidable lag.
- **Idle waste:** the relay wakes every interval even when there's nothing to do.

The fix is an adaptive loop, not a new mode.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Relay** | **Drain-until-empty**: keep calling `run_once` with no sleep while a tick returns a full `BATCH` (more likely pending); only sleep when a tick drains `< BATCH` (caught up). Then **idle backoff**: grow the sleep up to a configured cap while idle, reset to the fast interval on the next non-empty tick. |
| **Relay (optional)** | An in-process enqueue nudge so a freshly written outbox row wakes the relay promptly — using a polling-safe primitive (mpsc / a polled flag), **not** `tokio::sync::Notify::notify_waiters` (banned for producer→poller signaling, see [[Decisions]]). |
| **Tests** | Backlog drains in bounded ticks (no fixed-interval stalls); idle relay's interval grows to the cap; activity resets it. |
| **Docs** | [[Production]] outbox relay cadence/backoff tuning. |

## Non-goals

- Changing at-most-once NOTIFY semantics or the quarantine path ([[Clusters/Cluster 12.0]]).
- The polled-vs-notify **mode** selection — already shipped in [[Clusters/Cluster 84.0]].
- Multiple relays / cross-process relay coordination — one relay per process remains correct.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 108.0.1 | `feat(server): drain-until-empty + idle backoff in outbox relay` |
| 108.0.2 | `feat(server): in-process enqueue nudge (polling-safe)` |
| 108.0.3 | `test(server): outbox_relay_backlog_drains_and_idle_backs_off` |
| 108.0.4 | `docs(production): outbox relay cadence tuning` |
| 108.0.retro | `docs(retro): Cluster 108.0 + v108.0.0 tag prep` |

## Exit criteria

- A backlog of N rows drains in ≈⌈N/BATCH⌉ ticks with **no fixed-interval stall** between full batches.
- An idle relay backs off to the configured cap and resets on the next pending row.
- At-most-once semantics, metrics, and quarantine behavior unchanged.
- `v108.0.0` tagged after retro.

## Ordering & risks

- **Independent** — can run in parallel with the rest of Phase XX.
- **Risk — starving a slow trickle:** cap the backoff so a row enqueued during a long idle window is still relayed within the cap interval; always reset to fast on activity.
- **Risk — signaling pattern:** do **not** reach for `Notify::notify_waiters` (misses current-only waiters); use a polling loop or mpsc per the [[Decisions]] rule and the `LoggingHandler::wait_for` precedent.

## References

- [[Clusters/Product Ladder 102+]] Phase XX
- [[Clusters/Cluster 84.0]] (relay modes), [[Clusters/Cluster 10.0]] / [[Clusters/Cluster 12.0]] (outbox + quarantine)
- [[Decisions]] (no `notify_waiters`), [[Production]], [[Architecture]]
