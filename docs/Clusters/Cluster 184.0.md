# Cluster 184.0 — correctness: harden the domain-write → event-append dual write

**Theme:** Arc A (security & correctness), finale — stop silently losing domain
events when the log append fails after the domain row has committed.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v184.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Distinguish durable-append failure from benign bus-publish failure; retry the append on transient errors; count hard losses | `routes/mod.rs::publish` |
| `maidan_event_append_failures_total` metric | `metrics.rs` |
| Generic `retry` helper + unit tests | `routes/mod.rs` |

## Why

Every mutation flows through `publish()`: the domain row is committed by the
store call, then `publish()` appends the `Event` in a **separate** transaction.
The old code logged a single `warn` and returned `None` on append failure — and
the callers ignore that `None` (fire-and-forget), so the HTTP caller got a `2xx`
while the event was **silently lost**: no WS/MCP notification, no at-least-once
delivery, no indexing. The two failure modes were also conflated: a bus-publish
hiccup (benign — the event is durably logged, and in relay mode the outbox
delivers it) was treated the same as a durable-append failure (an actual lost
event).

## The fix (and its honest boundary)

Full single-transaction atomicity — the domain mutation and the event append in
**one** DB transaction (a transactional outbox) — is the ideal, but it is a
multi-cluster refactor: every mutating `Store` method (× both backends) would
have to append its event in-tx, and the message path additionally does
post-insert edits (slash-command dispatch) *before* it publishes, so it is not
even a single clean insert+event. That is deferred and tracked in [[Open Work]].

What this cluster does, uniformly across every mutation via the shared seam:

- **Retry the durable append** on transient store errors (3 attempts, 50 ms
  backoff) — the common failure (pool timeout, brief lock contention) usually
  clears, converting most would-be silent losses into successes. The first
  attempt stays on the hot path (no clone); only a failure enters the retry loop.
- **Distinguish the failure modes**: a bus-publish failure stays a `warn`
  (benign); a durable-append failure after retries is a loud `error!`
  (`event.append_failed`) **and** increments
  `maidan_event_append_failures_total`, so a lost event is alertable instead of
  invisible.

## Exit criteria

- A transient append failure no longer loses the event (it's retried); a hard
  loss after retries is logged + metered; bus failures remain benign — **met**.
- `v184.0.0` tagged.

## Verification & limits

- `publish_tests::retry_returns_ok_after_transient_failures` /
  `retry_gives_up_and_returns_last_error` (unit, via a generic counter closure —
  no store mock needed). Existing `publish_tests` (relay on/off) unchanged.
- **Limit (tracked):** this is a *mitigation*, not atomicity. A process crash
  between the domain commit and a successful append still loses the event, and a
  hard store outage past the retries still drops it (now metered). True
  single-tx atomicity is the deferred follow-up.

## References

- [[Retros/Cluster 184.0]]; `routes/mod.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Arc A finale). Deferred atomicity: [[Open Work]].
