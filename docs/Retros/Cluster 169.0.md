# Cluster 169.0 retro — coalesce optimistic-path delivery-cursor writes

> Tag **`v169.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 2 (perf), part 4 — closes the DB-hot-path perf items.

## What shipped

- **H2:** `forward_bus_items` (the optimistic subscribe path) no longer writes
  the delivery cursor on every event. It buffers the highest delivered `log_id`
  and persists it at most once per 64 events or 500 ms
  (`CURSOR_FLUSH_EVENTS` / `CURSOR_FLUSH_INTERVAL`), plus a final
  `flush_delivery_cursor` when the stream ends.
- `replay_matching_events` (lag replay) now advances the cursor once to the
  batch high-water instead of per replayed row.

## What was deferred / not covered

| Item | Why |
|------|-----|
| `reconcile_deliver` | Already advances once per poll-batch — nothing to coalesce. |
| CI/CD workflow speedups | Next cluster (170). |

## Surprises

- **The authoritative path was already batched.** The at-least-once poller
  (`reconcile_deliver`, Cluster 125) advances the cursor once per drained batch.
  The per-event write was on the *optimistic* path, where the cursor is only
  best-effort (maintained so a later at-least-once reconnect doesn't re-deliver
  the whole history). That framing is what makes coalescing obviously safe here:
  worst case is a few re-delivered events, never a skip.

## Decisions

- **Check elapsed on each event; no separate timer task.** The flush condition is
  evaluated when an event arrives (`count >= 64 || last_flush.elapsed() >= 500ms`)
  plus a flush on loop exit. No `select!` timer — an idle subscriber's last few
  events wait for the next event or a clean disconnect (which flushes). The only
  unflushed-on-crash window re-delivers, matching the contract.
- **Monotonic advance makes ordering-with-replay a non-issue.** If a lag replay
  advances the cursor past a still-buffered optimistic value, the later flush of
  the lower value is a no-op — no clearing/coordination needed.

## Capability table extension

| Fix | Where |
|-----|-------|
| Coalesced optimistic delivery-cursor writes (count/time debounce + flush; replay advances once to high-water) | `crates/maidan-server/src/event_stream.rs` |

## Risks identified + still open

- **Low.** Best-effort cursor on a path whose durability is provided by the
  reconcile loop; monotonic + at-least-once means no skips. Delivery e2e
  (reconnect-cursor floor, resume-token reconnect) green.

## Forward look

Arc 2's code-perf items (R1/R2/R3, H1/H4/H6, H2) are **done**. What remains in
arc 2 is the **CI/CD workflow speedups** (Cluster 170): a native arm64 release
runner to kill the ~2 h QEMU-emulated image build, `gha` cargo caching, a
`trivy` image scan, and building the smoke image once for reuse across the Docker
jobs. Then arc 3 (agentic features).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
