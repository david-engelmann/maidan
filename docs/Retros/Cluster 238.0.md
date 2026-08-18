# Cluster 238.0 retro — mentions become deliverable

> Tag **`v238.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 2** — Arc G.

## What shipped

- `NotificationRouter` — an always-on, reconnecting event-bus consumer (the webhook
  worker's skeleton) spawned in `main.rs` and drained on shutdown.
- `route_event`: `MentionRecorded` → a `maidan_notifications` row for the mentioned
  member, with the channel resolved from the thread.
- `create_notification_if_absent` + a `UNIQUE(member_id, source_log_id)` index (pg
  0043 / sqlite 0042) — cross-replica/replay-idempotent writes.
- `maidan_notifications_created_total{kind}` metric.

An @mention was recorded and pollable but never *delivered* to a per-recipient
place; now it lands in the recipient's ledger the moment the event hits the bus. The
unified inbox (239) reads it; the MCP long-poll (240) waits on it.

## Surprises / decisions

- **The dedup index is load-bearing, and it's about replicas, not just replays.**
  It's tempting to skip dedup for a "just insert a row" feature, but every server
  replica runs the bus consumer, so a single mention event fans to *N* routers, each
  trying to write the same recipient row. Without the `UNIQUE(member_id,
  source_log_id)` index + `ON CONFLICT DO NOTHING`, a three-replica deployment would
  triple every notification. The unique key is exactly (recipient, source event),
  which is also the right semantic — one inbox entry per event per person. The metric
  only counts real writes, so it doesn't over-report on the replicas that lose the race.
- **Always-on, not opt-in.** The retention and scheduler sweepers are env-gated
  because they're optional ops behaviours; the notification router is the *product* —
  gating it would mean notifications silently don't exist unless someone sets a flag.
  It's safe to run unconditionally because it only writes additive rows and the
  worker only spawns in the server binary (`main.rs`), so `AppState`-embedding tests
  and the smoke jobs are untouched.
- **Test the pure resolver, not the async loop.** `route_event(state, log_id,
  event)` holds all the logic (match the kind, resolve the channel, dedup-insert), so
  the e2e calls it directly — deterministic, no "spawn the worker, publish, poll with
  a timeout" flake. The reconnect/consume loop is the same shape as the webhook
  worker's, already proven.
- **`MentionRecorded` lost its actor on the way here.** The event carries the
  mentioned member but not the mentioner, so `actor_id` is `None` for now. When Arc H
  adds richer routing (and if the actor becomes worth surfacing), it can be threaded
  through the event — a deliberate small gap, not an oversight.

## Capability table extension

| Change | Where |
|--------|-------|
| `NotificationRouter` + `route_event` (mentions) + `create_notification_if_absent` + dedup index + metric | `notification_router.rs`, `store/*/notifications.rs`, `migrations/*`, `metrics.rs`, `main.rs` |

## Risks identified + still open

- None new. The dedup index closes the multi-replica double-notify risk; migration
  registration is covered by the parity tests.

## Forward look

Arc G finishes: **239** the REST unified inbox (`GET /members/:id/notifications` +
unread-count + mark-read/read-all, self-only per the Cluster-202 model), **240** the
MCP tools + a `wait_for_notification` long-poll (the `wait_for_mention`
generalization — now backed by a durable ledger, not a live-only bus subscribe).
Then Arc H (preferences / mute / follow — `route_event` is the extension point) and
Arc I (email/SMTP transport, digests, presence-aware routing, `/ui` center).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 237.0]].
