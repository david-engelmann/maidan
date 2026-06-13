# Cluster 108.0 retro — Adaptive outbox relay

> Tag **`v108.0.0`**. Third cluster of Phase XX (hot-path hardening).

## What shipped

- **Adaptive cadence** — `run_once` returns `RelayTick { fetched, relayed }`; `run()` drains back-to-back while a tick fully relays a batch (`relayed == BATCH`), then sleeps the base interval when caught up and **backs off ×2** toward `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS` (default 1000 ms) while idle, resetting on the next pending row. A partial/failing tick falls through to the base sleep, so a stuck row can't hot-spin. (108.0.1, #300)
- **Enqueue nudge** — `AppState.outbox_nudge` (capacity-1 mpsc); `publish` pings it after enqueuing a row, and the relay's idle wait is a cancel-safe `select!` over the backoff timer and `rx.recv()` (`wait_idle_or_nudge`). A nudge wakes the relay immediately and resets the cadence; a closed channel is dropped so it can't spin. (108.0.2, #301)
- **Docs** — `docs/Production.md` outbox relay cadence/backoff tuning + the new `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS`. (108.0.4, this PR)

## What was deferred / not covered

- **108.0.3** (a dedicated backlog/backoff e2e) folded into the unit tests added in 108.0.1/.2 (`backlog_drains_in_bounded_batches_then_idles`, `idle_backoff_doubles_to_cap`, the three `*_nudge_*` tests) — they cover the exit criteria directly and deterministically, so a separate slow e2e wasn't worth it.
- At-most-once NOTIFY semantics, the quarantine path, and the polled-vs-notify **mode** were explicitly out of scope (unchanged).
- Cross-process relay coordination — one relay per process stays correct.

## Surprises

- **Hot-spin trap.** A naïve "drain while a full batch was *fetched*" would busy-loop when a whole batch of rows keeps failing (they stay pending until quarantined). Gating the no-sleep path on `relayed == BATCH` (actual progress) instead means failures fall through to the base-interval sleep — backlog still drains around a stuck row, but the loop never spins.
- **Closed mpsc spins too.** A closed channel's `recv()` resolves *instantly, forever*, so a `select!` on it would busy-loop the moment the sender drops. `wait_idle_or_nudge` takes the receiver (`*nudge = None`) on close and falls back to a plain sleep.

## Decisions

- **mpsc nudge, never `Notify::notify_waiters`.** Per the standing [[Decisions]] rule (and the `LoggingHandler::wait_for` precedent), producer→poller signaling uses a polling-safe primitive. A capacity-1 mpsc is exactly right: a `Full` `try_send` means a wake is already pending; the relay drains *all* pending rows on the next tick, so one nudge per burst suffices.
- **Adaptive loop, not a new mode.** The cadence shape is a property of the existing relay, gated on tick outcome — no new `MAIDAN_OUTBOX_RELAY_MODE`.

## Capability table extension

| Capability | Where |
|------------|-------|
| Drain-until-empty + idle backoff relay cadence | `OutboxRelay::run`, `RelayTick`, `backoff_step` |
| Prompt wake on enqueue (polling-safe nudge) | `AppState.outbox_nudge`, `OutboxRelay::with_nudge`, `wait_idle_or_nudge` |

## Risks

- A missed nudge (e.g. the capacity-1 channel was already full and then drained between ticks) only ever delays a row by at most the current idle interval (≤ cap); correctness is unaffected.
- Too-large a cap lengthens worst-case enqueue→publish latency if the nudge is ever unwired (e.g. relay disabled in a non-prod dev path); the default 1000 ms is conservative.

## Next

Cluster **109** — ANN index tuning (HNSW `m` / `ef_*` configurable) + a `criterion` bench harness for lexical + semantic latency (Track U).
