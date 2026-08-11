# Cluster 186.0 retro — the growth tables finally have a ceiling

> Tag **`v186.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc B (multi-tenant SaaS operability), part 2.

## What shipped

- Store-level batched pruning for the event log, audit trail, and delivery
  tables on both backends, plus `min_delivery_cursor` (the event-log safety
  floor), and an opt-in background sweeper with config + a
  `maidan_retention_pruned_total{table}` metric.

## Surprises

- **The event log's safe cutoff is a *cursor*, not a clock.** Age alone is unsafe
  — a durable at-least-once consumer that's lagging could still need an "old"
  event. So the events prune floors at `MIN(last_delivered_log_id)` across all
  cursors; only rows delivered to *every* durable consumer are eligible. The age
  cutoff still applies on top. Getting this wrong would silently break the
  at-least-once guarantee the whole delivery subsystem exists to provide.
- **Timestamps you can't set make tests awkward.** `append_audit` stamps
  `occurred_at = now()`, so there's no public way to insert an *old* audit row —
  the audit test proves the cutoff by pruning with a future vs past cutoff
  instead. Events were testable directly (`append_event` takes `occurred_at`),
  which is exactly why the risky event-log path got the thorough test.
- **My own test tripped the global-table gotcha.** `maidan_events` isn't
  workspace-scoped for pruning, so the first scenario's leftover rows leaked into
  the second's counts (a `future` cutoff swept a prior recent event under the
  floor). Fixed by seeding genuinely-old events and using an age cutoff — the
  code was right; the test's assumption wasn't.

## Decisions

- **Batched deletes (subquery `LIMIT`), loop until short.** A single
  `DELETE ... WHERE occurred_at < cutoff` on a table that's never been pruned
  could delete millions of rows under one lock. The `id IN (SELECT ... LIMIT n)`
  form is identical across both backends and keeps each statement bounded.
- **Opt-in, off by default.** Silent data deletion is not a default. No
  `MAIDAN_RETENTION_*_DAYS` → the sweeper isn't even spawned.
- **Deliveries: terminal-only, light test.** Pruning in-flight deliveries would
  drop undelivered work, so only `delivered_at`/`quarantined_at` rows are
  eligible. The DELETE mirrors audit's (age) plus that status filter; I covered
  it with a valid-query/empty smoke rather than build the webhook-subscription FK
  fixture — and said so.

## Capability table extension

| Change | Where |
|--------|-------|
| Opt-in age retention for events (cursor-floored) / audit / deliveries + sweeper | `maidan-store` + `maidan-server/src/retention.rs` |

## Risks identified + still open

- **Net risk-reducing, opt-in.** Open/tracked: no `occurred_at` index (daily
  batched sweep tolerates a scan; add if it gets hot); deliveries prune lightly
  tested; a stuck/abandoned delivery cursor pins the event-log floor (a durable
  consumer that vanishes without cleaning its cursor blocks pruning of newer
  events — safe, but needs a stale-cursor reaper eventually).

## Forward look

Arc B continues: workspace export/portability, per-tenant metrics/metering, and
a secret-rotation keyring.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
