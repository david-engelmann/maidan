# Cluster 125.0 — At-least-once event delivery

**Theme:** Close the silent out-of-order delivery gap with an opt-in,
cursor-driven at-least-once subscribe mode. Per-stream dedup was already handled
(watermark + cursor); the real hole was *completeness*.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v125.0.0`**, no new
gate tag.

**Predecessor:** the event log + delivery cursor (13, 56, 83), the optimistic
live path + replay-on-lag (`forward_bus_items`).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Store (125.0.1)** | `maidan_events.inserted_at` (insert wall-clock) + `Store::list_events_after_stable` — the stability-gated, gap-safe read. |
| **Delivery (125.0.2)** | `event_stream::reconcile_deliver` + the `at_least_once` subscribe flag; `ws.rs` branches reconcile vs the unchanged optimistic path. Config on `AppState` from env. |
| **Docs (125.0.3)** | ADR in `Decisions.md`; Production.md env + the at-least-once contract (guarantee, latency cost, long-transaction caveat). |

## Non-goals

- Changing the default (optimistic) live path — `at_least_once` is opt-in;
  everyone else is byte-for-byte unchanged.
- MCP SSE (`/mcp/stream`) parity — this cluster wires WebSocket; MCP is a clean
  follow-on.
- Strictness against a write transaction longer than the stability window
  (documented caveat; size the window above the slowest writer).

## PR ladder (actual)

| # | Title |
|---|--------|
| 125.0.1 | `feat(store): inserted_at + stability-gated event replay` (#341) |
| 125.0.2–3 | `feat(ws): at-least-once reconcile delivery for subscriptions` (#342) |
| 125.0.retro | `docs(retro): Cluster 125.0 + v125.0.0 tag prep` |

## Exit criteria

- An `at_least_once` (`workspace_id + consumer_id`) subscription delivers every
  committed matching event in `log_id` order, exactly once per consumer, with no
  silent out-of-order gap — **met** (e2e: ordered backlog + cursor-floored
  reconnect).
- The optimistic path is unchanged for non-opting subscribers — **met** (full
  `ws_subscribe_e2e` suite unchanged, 11/11).
- `v125.0.0` tagged after retro.

## Ordering & risks

- **Foundation first (0.1), behavior change second (0.2).** The store primitive
  is additive and was merged + Postgres-verified before the consumer-loop change.
- **Stability horizon assumption.** Correctness holds under "no insert
  transaction outlives the window"; the `inserted_at <= now - W` gate ensures a
  lower `log_id` is visible+stable before a higher one is delivered.
- **Test determinism.** The reconcile advances the durable cursor just after the
  batch send; the e2e waits for that commit before reconnecting (a race that
  would otherwise read a benign at-least-once re-delivery).

## References

- [[Retros/Cluster 125.0]]; [[Decisions]] ("At-least-once delivery via cursor reconciliation…")
- `crates/maidan-server/src/event_stream.rs` (`reconcile_deliver`), `crates/maidan-store/src/*/events.rs`
- [[Production]] ("At-least-once delivery")
