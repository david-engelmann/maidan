# Cluster 238.0 — notification router (events → per-recipient rows)

> **Program C (notifications & reach), part 2** — Arc G. Phase XXIV post-gate
> hardening. Tag **`v238.0.0`**. No new gate tag.

## Goal

Turn the Cluster-237 ledger from an empty table into a live per-recipient feed: a
background bus consumer that resolves each relevant event to the members it concerns
and writes a `maidan_notifications` row for each. This cluster routes @mentions (the
mentioned member); preferences, follows, and more event kinds layer on later.

## Scope

| Change | Where |
|--------|-------|
| `NotificationRouter` — a reconnecting event-bus consumer, spawned in `main.rs`, drained on shutdown | `notification_router.rs`, `lib.rs`, `main.rs` |
| `route_event` — `MentionRecorded` → a notification for the mentioned member (channel resolved from the thread) | `notification_router.rs` |
| `create_notification_if_absent` (ON CONFLICT DO NOTHING) + a `UNIQUE(member_id, source_log_id)` index (pg 0043 / sqlite 0042) | `store/*/notifications.rs`, `store.rs`, migrations, `migrate.rs` |
| `maidan_notifications_created_total{kind}` metric | `metrics.rs` |

## Design decisions

- **Cross-replica dedup is a hard requirement, not a nicety.** Every server replica
  runs the router's bus consumer, so the *same* event (same `log_id`) reaches every
  replica — a naive insert would write N duplicate rows. A `UNIQUE(member_id,
  source_log_id)` index + `create_notification_if_absent` (`ON CONFLICT DO NOTHING
  RETURNING`) makes the write idempotent across replicas *and* event replays. The
  router increments the metric only when a row was actually written (`Some`).
- **Always-on, like the webhook worker.** The router is the delivery backbone of
  Program C, not an ops toggle — it spawns unconditionally in `main.rs` (no env
  flag). It's additive: it only writes rows, so tests/embedders that build
  `AppState` without spawning workers are unaffected, and the smoke jobs (which don't
  mint mentions) see no behavioural change.
- **Resolve the channel from the thread.** `MentionRecorded` carries no
  `channel_id`, but the inbox wants it for rendering + future RBAC scoping — one
  best-effort `get_thread` lookup fills the denormalized column.
- **Mirror the webhook worker's shape.** A `watch`-channel shutdown, a reconnecting
  `subscribe → consume` loop with exponential backoff, `Lagged`-marker logging — the
  proven bus-consumer skeleton, minus the delivery poller (the ledger *is* the
  durable record; there's no outbound retry).

## Non-goals / deferred

- **REST inbox** (Cluster 239) + **MCP tools + `wait_for_notification`** (240).
- **More event kinds + preferences/follows** (Arc H) — the router handles only
  `MentionRecorded` today; `route_event` is the single extension point.

## Risks

- Migration registration (the standing gotcha) — covered by both-backend store
  tests + `dialect_parity` / `concurrent_migrations`. The router's core logic is
  unit-tested directly (`route_event`) rather than through the async worker loop, so
  there's no timing flake.
