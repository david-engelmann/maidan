# Cluster 128.0 retro — A2A delivery robustness

> Tag **`v128.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. Second
> cluster of the hardening sweep (127 reconcile → **128 A2A** → 129 → 130).

## What shipped

- **A2A client timeouts** (`maidan-a2a/src/client.rs`): the `A2aClient` builder
  set no timeout, so any request could hang indefinitely. Added a 10s
  `connect_timeout` (bounds the connect hang for every request, streaming
  included, without capping a legitimately long stream) + a 30s per-request
  timeout on the non-streaming `call`.
- **A2A push retry/backoff/visibility** (`a2a_agent.rs::deliver_a2a_push`): the
  push POST was `let _ = client.post(...).send()` — silent on failure. Now 3×
  retry with capped exponential backoff, per-attempt logging, and a
  `maidan_a2a_push_total{result=ok|failed}` counter.
- **SSE error visibility**: the subscribe poll logs the `load_task` error that
  previously ended the stream silently; the SSE-frame serializer logs on failure
  instead of emitting a silent empty frame.
- **Tests**: an in-file axum harness (fail twice → 200) asserting retry-to-
  success and give-up-at-max-attempts (no hang/loop).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Durable A2A push outbox | The retry is best-effort; transactional durability is a larger effort, only warranted if push becomes a hard guarantee. |
| 129 | The non-A2A hardening findings (unbounded MCP channel, outbox quarantine swallow, unreachable!(), Prometheus thread) | Grouped into the next cluster. |

## Surprises

- **Two different failure modes in one path.** The A2A delivery had both a
  *hang* risk (no client timeout) and a *silent-drop* risk (fire-and-forget
  push). They needed different fixes — a connect timeout for the former, retry +
  visibility for the latter — not one knob.
- **Testing a spawned best-effort path.** `deliver_a2a_push` is private and
  spawned; the clean test was an in-process axum endpoint with a hit-counter
  that fails N times, letting the test assert the exact retry count rather than
  mocking time.

## Decisions

- **Best-effort + visible, not durable.** A bounded retry with logging + a metric
  is the right weight for a push notification — it turns silent drops into
  observable ones without the cost of a transactional outbox. Documented as
  best-effort so no one mistakes it for a guarantee.
- **`connect_timeout` for streaming, full timeout for unary.** Avoids cutting a
  legitimately long streaming response while still bounding the hang.

## Capability table extension

| Capability | Where |
|------------|-------|
| A2A push retry + backoff + `maidan_a2a_push_total` metric | `crates/maidan-server/src/a2a_agent.rs` |
| A2A client connect/request timeouts | `crates/maidan-a2a/src/client.rs` |

## Risks identified + still open

- **Push is still best-effort.** A repeatedly-unreachable subscriber loses the
  notification after 3 attempts (logged + counted). Durable delivery would need
  an outbox.

## Forward look

Next in the sweep: **Cluster 129** — error-visibility + bounded buffers (the
unbounded MCP streamable channel, the swallowed outbox quarantine error,
`unreachable!()` → handled, the unsupervised Prometheus thread).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
