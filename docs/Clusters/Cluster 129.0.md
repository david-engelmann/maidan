# Cluster 129.0 — Hardening: error-visibility + bounded buffers

**Theme:** Fix the highest-impact correctness/robustness findings from the v126
hardening scan — an unbounded buffer (memory-exhaustion risk), a swallowed error
(infinite-retry risk), and request-handler `unreachable!()` panics.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v129.0.0`**, no new
gate tag. Third cluster of the hardening sweep (127 → 128 → **129**).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **MCP streamable** | `unbounded_channel()` → bounded `channel(256)` + non-blocking `try_send`; a full buffer logs + disconnects the slow client. |
| **Outbox relay** | A failed `quarantine()` is logged (next tick retries) instead of `let _ = …` (which left the row pending → infinite retry). |
| **Request handlers** | `unreachable!()` → typed errors in `delivery_ops` (get/replay) + `mcp/resources` read. |
| **Test** | Filling the bounded buffer fails the next push without blocking. |

## Non-goals

- Supervising the Prometheus upkeep `std::thread` — it's a process-lifetime
  daemon; converting it is churn without real benefit (explicitly deferred).
- The MCP/observability coverage uplift — that's Cluster 130.

## PR ladder (actual)

| # | Title |
|---|--------|
| 129.0.1 | `fix: harden error-visibility + bound the MCP streamable buffer` (#349) |
| 129.0.retro | `docs(retro): Cluster 129.0 + v129.0.0 tag prep` |

## Exit criteria

- No unbounded per-session buffer; no swallowed quarantine error; no
  `unreachable!()` in a live request handler — **met**.
- `v129.0.0` tagged after retro.

## Ordering & risks

- **Bounded buffer must use `try_send`.** `push` holds the registry mutex; an
  `await` on capacity would stall every session. `try_send` + fail-on-full is the
  correct non-blocking backpressure and matches the existing `push`→false
  "session gone" contract.
- **Drop-on-full is a deliberate trade.** A slow MCP client is disconnected (and
  must reconnect + re-sync) rather than consuming unbounded memory.

## References

- [[Retros/Cluster 129.0]]; v126 hardening scan
- `crates/maidan-mcp/src/streamable_session.rs`, `crates/maidan-server/src/outbox_relay.rs`, `delivery_ops.rs`, `crates/maidan-mcp/src/resources.rs`
