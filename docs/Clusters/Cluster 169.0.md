# Cluster 169.0 — perf: coalesce optimistic-path delivery-cursor writes

**Theme:** Arc 2 (perf), part 4 — H2, the last of the DB-hot-path perf items.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v169.0.0`**, no new gate tag.

---

## Scope

| Fix | Where |
|-----|-------|
| Coalesce the per-event delivery-cursor write on the optimistic live path (count/time debounce + flush on stream end) — H2 | `server/event_stream.rs` (`forward_bus_items`) |
| Coalesce the per-row cursor write in lag-replay into one advance to the batch high-water — H2 | `server/event_stream.rs` (`replay_matching_events`) |

## Why

- **H2:** `forward_bus_items` (the *optimistic* subscribe path) issued a
  synchronous `advance_delivery_cursor` **DB UPSERT on every delivered event**,
  in the hot path before the frame is even sent — one write per event per
  subscriber. That cursor is only *best-effort* on this path (the authoritative
  at-least-once path is `reconcile_deliver`, which already advances once per
  batch). It now buffers the highest delivered `log_id` and persists it at most
  once per `CURSOR_FLUSH_EVENTS` (64) or `CURSOR_FLUSH_INTERVAL` (500 ms),
  whichever first, plus a final flush when the stream ends.
- The lag-replay path (`replay_matching_events`) likewise advanced per replayed
  row; it now advances once to the batch high-water — mirroring what
  `reconcile_deliver` already does.

## Correctness

`advance_delivery_cursor` is **monotonic**, and delivery is **at-least-once**: a
coalesced-away write only means a subsequent at-least-once reconnect re-delivers
a few already-seen events, which the contract already tolerates (consumers dedup
on `log_id`). The flush-on-exit keeps a clean disconnect fully consistent — the
only lossy window is an unclean crash, and that re-delivers, never skips. The
gap-free guarantee of the reconcile path is untouched (not modified).

## Non-goals

- `reconcile_deliver` — already batches per-poll; left as-is.
- CI/CD workflow speedups — next cluster (170).

## Exit criteria

- Optimistic path writes the cursor on a threshold + flush, not per event;
  delivery e2e (reconnect-cursor contract) green — **met**.
- `v169.0.0` tagged.

## Verification & limits

- Delivery e2e suites (`ws_subscribe_e2e` incl. the resume-token reconnect +
  at-least-once cursor-floor tests, `mcp_stream_at_least_once_e2e`,
  `ui_ws_tail_e2e`) exercise the reconnect-cursor contract on both transports.
- Limit: a low-traffic subscriber that delivers `< 64` events and then sits idle
  keeps its last few in-memory until the 500 ms interval or disconnect — a
  deliberate best-effort trade (the reconcile path is the durable one).

## References

- [[Retros/Cluster 169.0]]; `server/event_stream.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program`.
