# Cluster 168.0 — perf: outbox relay round-trips + tunable broadcast cap

**Theme:** Arc 2 (perf), part 3 — the outbox relay's per-row round-trips and
the fixed broadcast-channel capacity. Also a main-red hotfix.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v168.0.0`**, no new gate tag.

---

## Scope

| Fix | Where |
|-----|-------|
| Outbox `list_pending` JOINs the event payload; relay publishes from it (no per-row `get_stored_event`) + batch `mark_published` — H4 | `store/{postgres,sqlite}/outbox.rs`, `server/outbox_relay.rs` |
| Env-tunable broadcast-channel capacity (`MAIDAN_BUS_BROADCAST_CAP`) — R1 | `bus/lib.rs` + `postgres.rs` / `inmem.rs` / `presence_notify.rs` / `resource_notify.rs` |
| **Hotfix:** remove two `unwrap()`s in `webhook_worker.rs` (Cluster 166) | `server/webhook_worker.rs` |

## Why

- **H4:** the relay did **three** round-trips per pending row —
  `get_stored_event(log_id)` to fetch the payload, `publish`, then a per-row
  `mark_published`. The payload already lives in `maidan_events`, so
  `list_pending` now JOINs it in (one SELECT for the whole batch instead of one
  per row), and the successfully-published rows are marked in a **single**
  `mark_published_batch` after the loop. A 64-row batch drops from ~128 extra
  DB calls to ~1.
- **R1:** every broadcast channel (event bus + presence/resource notifiers) was
  hard-coded to 1024. A slow subscriber that lags past the cap gets
  `RecvError::Lagged` and drops frames; operators had no knob. It's now
  `MAIDAN_BUS_BROADCAST_CAP` (default 1024) via a shared
  `broadcast_cap_from_env()`.
- **Hotfix:** Cluster 166's lazy-payload change left `payload.as_deref().unwrap()`
  in library code — a CLAUDE.md violation that the `lint` job's dedicated
  `-D clippy::unwrap_used` step rejects. It merged during the GitHub Actions
  outage (validated only with `--all-targets -D warnings`, which does **not**
  enable that restriction lint), so `main` went red the moment CI recovered.
  Rewritten with `let-else` / `if let Some`.

## Non-goals

- H2 (delivery-cursor coalesce) — next cluster; it touches the per-subscriber
  cursor write path, not the outbox.
- CI/CD workflow speedups — next cluster (now unblocked; CI recovered).

## Exit criteria

- Relay publishes from the JOINed payload + batch-marks; broadcast cap is
  env-tunable; `main` green under the strict lint; suites green — **met**.
- `v168.0.0` tagged.

## Verification & limits

- `list_pending_joins_the_event_payload` + `mark_published_batch_clears_all_pending_and_is_idempotent`
  (Postgres). The relay e2e suites (`outbox_relay_e2e`, `outbox_polled_relay_e2e`,
  `outbox_http_e2e`, `outbox_sqlite_http_e2e`) exercise the batch path end-to-end
  on both backends.
- Limit: batch `mark_published` widens the duplicate-republish window on a crash
  between publish and the batch mark (all N in the batch, vs. 1 before). The
  at-least-once contract already tolerates duplicates (consumers dedup on
  `log_id`), so this is a latency/throughput win, not a correctness change.
- Validated locally with **both** clippy invocations this time (see
  [[Retros/Cluster 168.0]]).

## References

- [[Retros/Cluster 168.0]]; `store/postgres/outbox.rs`, `store/sqlite/outbox.rs`,
  `server/outbox_relay.rs`, `bus/lib.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.
