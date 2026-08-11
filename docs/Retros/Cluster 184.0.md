# Cluster 184.0 retro — a lost event is no longer silent

> Tag **`v184.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), **finale**.

## What shipped

- `publish()` now retries the durable event append on transient store errors
  (3 attempts, 50 ms backoff), distinguishes an append failure (dangerous — the
  event is lost) from a bus-publish failure (benign — already logged), and on a
  hard append failure logs `event.append_failed` + increments
  `maidan_event_append_failures_total`.
- A small generic `retry` helper, unit-tested without a store mock.

## Surprises / the scoping call

- **True atomicity was bigger than one cluster — and I said so instead of
  faking it.** The finding ("dual-write: domain row commits, event appends
  separately, swallowing failures") points at a transactional-outbox refactor:
  domain mutation + event append in one DB transaction. But (a) that's every
  mutating `Store` method × two backends, and (b) the flagship path —
  message-post — does *insert → slash-command edit → publish*, so the event
  isn't even emitted right after the insert. Wrapping just the insert would
  publish a stale pre-edit event. A partial refactor would leave a
  mixed-atomicity codebase that's harder to reason about than the current
  uniform best-effort. So I hardened the **shared seam** every mutation already
  flows through (uniform, no mixed state) and deferred true atomicity with a
  written plan — the same judgment-over-completion call as 181 (collapse vs
  guard) and 182 (declining 403 auditing).
- **The old `warn` conflated two very different failures.** A bus hiccup and a
  lost durable event were logged identically; splitting them is half the value
  here (you can now alert on the one that matters).
- **Async-closure lifetimes pushed the design toward a clean shape.** Passing
  `&event` through a generic `FnMut() -> Fut` retry helper fights the borrow
  checker; keeping the first attempt inline (borrow) and cloning only into the
  rare retry loop sidestepped it *and* kept the hot path clone-free.

## Decisions

- **Mitigate uniformly now, atomicity later.** Retry + observe closes the common
  transient-loss window for *all* mutations at once and makes hard losses
  alertable — real risk reduction today — while the correct-but-large single-tx
  work is tracked, not rushed.
- **Never 5xx a successful mutation.** The domain row committed; returning an
  error would be a lie and would invite duplicate retries. The lost-event signal
  goes to logs + metrics (for reconciliation/alerting), not the caller.

## Capability table extension

| Change | Where |
|--------|-------|
| Retried durable event append + lost-event metric/alert; append-vs-bus failure split | `routes/mod.rs`, `metrics.rs` |

## Risks identified + still open

- **Net risk-reducing, no behaviour change on success.** Open + tracked: true
  single-transaction atomicity (a crash between commit and append, or a hard
  store outage past the retries, still drops an event — now metered). This is the
  one Arc-A item that closes as *substantially mitigated + tracked* rather than
  *done*.

## Forward look

**Arc A (security & correctness) is complete** (179 A2A RBAC, 180 DM-thread
participant check, 181 EventKind parser unification, 182 audit coverage, 183
default-on limits, 184 dual-write hardening). Next: **Arc B — multi-tenant SaaS
ops** (Helm HA/liveness fixes, workspace export, data retention, per-tenant
metering, secret rotation).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
