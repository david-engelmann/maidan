# Cluster 126.0 retro — MCP SSE at-least-once parity

> Tag **`v126.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`at_least_once` on `/mcp/stream`** (126.0.1): a query param that routes an
  MCP SSE subscription (`workspace_id + consumer_id`) through the same
  `event_stream::reconcile_deliver` loop the WebSocket path uses — stability-
  gated, cursor-driven, gap-free, exactly-once per consumer. Skips the optimistic
  replay; window + cadence come from the Cluster 125 `AppState` config. Unset →
  the optimistic path is byte-for-byte unchanged.
- **SSE e2e** (`mcp_stream_at_least_once_e2e`): parses the `text/event-stream`
  body and asserts the stable backlog is delivered in `log_id` order and the
  durable cursor advances.
- **Docs**: Production.md's at-least-once contract now covers both transports.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Covered elsewhere | Cross-reconnect floor over SSE | Shared `reconcile_deliver` logic; asserted deterministically on WebSocket. Over SSE it's racy (see Surprises). |

## Surprises

- **A dropped SSE connection doesn't instantly stop the server task.** The first
  draft reconnected the same `consumer_id` to assert the cursor floor — but
  dropping the reqwest response doesn't synchronously tear down the server-side
  reconcile task, so the still-live first stream consumed the post-cursor event
  (advancing the cursor) before the reconnect. That's *correct* at-least-once
  behavior (the first consumer got it), just not what a naive reconnect test
  expects. Resolution: the MCP test asserts only what the SSE wiring uniquely
  proves (ordered backlog + cursor advance); the floor stays on the WebSocket
  e2e where close is synchronous.

## Decisions

- **Reuse, don't reimplement.** `reconcile_deliver` is transport-agnostic (it
  writes to an `mpsc<String>`); both `/ws/subscribe` and `/mcp/stream` feed it.
  Parity was a query param + a branch, nothing more.
- **Scope the SSE test to its unique value.** Don't re-prove shared logic over a
  flakier transport.

## Capability table extension

| Capability | Where |
|------------|-------|
| At-least-once on MCP SSE (`/mcp/stream?at_least_once=true`) | `crates/maidan-server/src/mcp_stream.rs` |

## Risks identified + still open

- **Same stability/latency trade as 125** — `MAIDAN_DELIVERY_STABILITY_SECS` is
  the fresh-event latency floor and the long-transaction safety margin.

## Forward look

Both real-time transports now offer opt-in gap-free at-least-once delivery.
The delivery-correctness arc (125–126) is complete; remaining standing risks
(e.g. exactly-once at the bus itself) are now largely addressed at the consumer.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
