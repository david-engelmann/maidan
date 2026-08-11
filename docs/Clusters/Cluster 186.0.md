# Cluster 186.0 — SaaS ops: data-retention pruning

**Theme:** Arc B (multi-tenant SaaS operability), part 2 — bound the
unbounded-growth tables (event log, audit trail, delivery queues) with opt-in
age retention, safely.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v186.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `min_delivery_cursor` + `prune_events`/`prune_audit`/`prune_deliveries` (batched, both backends) | `maidan-store/src/{sqlite,postgres}/retention.rs`, `store.rs` |
| Background retention sweeper + env config + `maidan_retention_pruned_total` | `maidan-server/src/retention.rs`, `metrics.rs`, `main.rs` |

## Why

`maidan_events` (one row per mutation), `maidan_audit`, and the webhook +
automation delivery tables grow without bound. For a long-lived multi-tenant
install that's an operational time-bomb (disk, vacuum, query latency). There was
TTL cleanup for sessions/oauth-codes/tokens but nothing for these.

## The fix

Opt-in age retention per table (`MAIDAN_RETENTION_{EVENTS,AUDIT,DELIVERIES}_DAYS`;
unset/`0` = keep forever), swept by a background loop every
`MAIDAN_RETENTION_SWEEP_SECS` (default daily), deleting in batches of
`MAIDAN_RETENTION_BATCH` (default 5000) so a first sweep over a huge table
doesn't take one giant lock.

- **Event-log safety.** Events are pruned only up to `min_delivery_cursor` — the
  lowest `last_delivered_log_id` across all at-least-once consumers — so a
  lagging durable consumer never loses an undelivered event. With no such
  consumer the floor is unbounded (prune purely by age). The age cutoff (days) is
  always far older than the delivery stability horizon (seconds), so that floor
  needs no separate check.
- **Deliveries** prune only **terminal** rows (`delivered_at` or `quarantined_at`
  set); in-flight/retrying rows are never pruned regardless of age.
- **Audit** prunes purely by `occurred_at`.

## Exit criteria

- Each table prunes by age; the event log respects the delivery watermark;
  everything opt-in and off by default — **met**.
- `v186.0.0` tagged.

## Verification & limits

- `maidan-store` `retention` test (SQLite + Postgres-testcontainers, same suite):
  age cutoff prunes the 100-day-old event and keeps the recent one; the
  cursor floor keeps an *old* event above the watermark while pruning the one
  at/under it; audit past-vs-future cutoff; `min_delivery_cursor` None→Some.
- `maidan-server` `retention::tests` (day parsing, cutoff arithmetic).
- Limits: **deliveries prune is covered by a valid-query/empty smoke** (its
  DELETE is structurally identical to audit's, plus a terminal-status `WHERE`);
  inserting delivery rows needs webhook-subscription FK setup, deferred. Optimistic
  reconnect replay beyond the retention window is out of scope by design.
  Retention days must exceed any indexer backlog (trivially true at day scale);
  no dedicated `occurred_at` index yet — the daily batched sweep tolerates a scan
  (logged as a follow-up if it becomes hot).

## References

- [[Retros/Cluster 186.0]]; `maidan-server/src/retention.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Arc B).
