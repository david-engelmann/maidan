# Cluster 237.0 retro — notifications get a per-recipient home (Program C opens)

> Tag **`v237.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 1** — opens Arc G.

## What shipped

- `maidan_notifications` (pg 0042 / sqlite 0041) + `Notification`/`NewNotification`
  (+ `NotificationId`) + store CRUD (create / list-for-member / mark-read /
  mark-all-read / unread-count), both backends. The per-recipient ledger a mention's
  shared row + single inbox cursor couldn't express. Storage only — no router,
  routes, or worker.

## Why Program C, and why this first

A 5-question recon of the current surface (mentions, delivery/webhooks, presence,
prefs, the subscribe substrate) confirmed the shape of the gap: a mention is
*recorded and polled*, never *delivered per-recipient*; webhook delivery is a single
per-workspace firehose keyed only on event kind; there are no preferences, no
mute/follow, and `deliver_http` is the only transport. The most foundational missing
piece — the thing every later capability (router, unified inbox, prefs, digests,
email) builds on — is a per-recipient notification row. So Program C opens exactly
where the last four foundations did (159 / 217 / 226 / 230 / 234): a table + model +
store CRUD with zero wiring.

## Surprises / decisions

- **Reuse `EventKind`, don't mint a `NotificationKind`.** The tempting move is a new
  display-oriented enum ("mention", "reply", "assignment"). But a notification's kind
  *is* the triggering event's kind, and a new enum would re-open the 11-site drill
  (enum + as_str + parse + ALL + accessors + contract + …) for zero new information.
  Reusing `EventKind` (stored as TEXT, `parse` on read) is both leaner and
  forward-compatible — any future event kind can notify without touching this table.
- **`source_log_id` is a pointer, not a foreign key.** It names the `maidan_events`
  row that triggered the notification, but a FK would couple the ledger to the event
  log's lifetime — and the log is retention-pruned (Cluster 186). The whole point of
  the denormalized `channel/thread/message/actor` columns is that a notification
  stays renderable after its source event ages out. So: no FK, keep the context.
- **`mark_read` via `COALESCE(read_at, now())`.** Two things fall out of one clause:
  a re-mark preserves the *first* read time (not the latest), and `rows_affected > 0`
  cleanly means "this id exists" — so the future REST route gets idempotency and a
  404 signal without a separate SELECT.
- **A partial index for the badge.** `idx_notifications_member_unread ... WHERE
  read_at IS NULL` keeps `unread_count` (the hot per-request badge query) cheap even
  as read history grows — both backends support the partial index.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_notifications` + `Notification`/`NewNotification` + store CRUD | `migrations/*`, `models.rs`, `store/*/notifications.rs` |

## Risks identified + still open

- None new. Migration registration (the standing "a `.sql` needs a `const` +
  `apply_*` call" gotcha) is covered by the both-backend store test plus
  `dialect_parity` / `backend_parity` / `concurrent_migrations`, all green.

## Forward look

Arc G continues: **238** the notification **router** (a bus consumer resolving events
→ recipients, writing rows; mentions first, dedup on `(member, source_log_id)`),
**239** the REST unified inbox (`GET /members/:id/notifications` + mark-read,
self-only per the Cluster-202 model), **240** the MCP tools + a `wait_for_notification`
long-poll (the `wait_for_mention` generalization). Then Arc H (preferences / mute /
follow) and Arc I (email/SMTP transport, digests, presence-aware routing, `/ui`
notification center).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 236.0]];
opens Program C after Program B (agentic orchestration) closed at 236.
