# Cluster 126.0 — MCP SSE at-least-once parity

**Theme:** Extend the Cluster 125 opt-in at-least-once delivery to the MCP SSE
transport (`GET /mcp/stream`), so MCP/SSE clients get the same gap-free
guarantee the WebSocket path already has.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v126.0.0`**, no new
gate tag.

**Predecessor:** Cluster 125 (`reconcile_deliver` + the `at_least_once` flag on
`/ws/subscribe`).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Delivery (126.0.1)** | `McpStreamQuery.at_least_once` query param; `/mcp/stream` branches to `reconcile_deliver` (vs the optimistic path) when set with `workspace_id + consumer_id`. |
| **Tests (126.0.1)** | SSE e2e: `at_least_once` stream delivers the stable backlog in `log_id` order and advances the durable cursor. |
| **Docs (126.0.1)** | Production.md at-least-once contract documents both transports. |

## Non-goals

- Changing `reconcile_deliver` itself — this is pure transport wiring reusing
  the Cluster 125 loop.
- Re-asserting the cross-reconnect floor over SSE — that is shared
  `reconcile_deliver` logic, covered deterministically by the WebSocket e2e
  (asserting it over a dropped SSE connection is racy and not worth the
  flake surface).

## PR ladder (actual)

| # | Title |
|---|--------|
| 126.0.1 | `feat(mcp): at-least-once parity for /mcp/stream SSE` (#344) |
| 126.0.retro | `docs(retro): Cluster 126.0 + v126.0.0 tag prep` |

## Exit criteria

- A `/mcp/stream` stream with `at_least_once=true` (+ `workspace_id` +
  `consumer_id`) is delivered by the reconcile loop, ordered, advancing the
  cursor — **met** (SSE e2e).
- The optimistic SSE path is unchanged when the flag is unset — **met**.
- `v126.0.0` tagged after retro.

## Ordering & risks

- **Pure wiring.** The risk is param routing, not delivery logic; the SSE path
  feeds the same `text_tx` whether reconcile or optimistic, so the transport is
  unchanged downstream.
- **SSE test parsing.** The e2e parses the `text/event-stream` body (split on
  `\n\n`, `data:` lines), skipping keep-alive comments; deterministic with
  `delivery_stability = 0` configured on `AppState`.

## References

- [[Retros/Cluster 126.0]], [[Retros/Cluster 125.0]]; [[Decisions]] (at-least-once ADR)
- `crates/maidan-server/src/mcp_stream.rs`, `crates/maidan-server/src/event_stream.rs` (`reconcile_deliver`)
- [[Production]] ("At-least-once delivery")
