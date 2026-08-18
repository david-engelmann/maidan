# Cluster 237.0 — per-recipient notification ledger (opens Program C)

> **Program C (notifications & reach), part 1** — opens **Arc G (per-recipient
> notification ledger + router + unified inbox)**. Phase XXIV post-gate hardening.
> Tag **`v237.0.0`**. No new gate tag.

## Goal

Open Program C with the **zero-blast-radius foundation** for per-recipient
notifications. Today an @mention is one shared `maidan_mentions` row read through a
single `maidan_inbox_cursor` — there is no per-member delivered/read state, and
"delivery" is either an ephemeral live bus frame or a single per-workspace webhook
firehose. This cluster lands the store + model for a per-recipient notification
ledger; the router, REST inbox, and MCP tools follow.

## Scope

| Change | Where |
|--------|-------|
| `maidan_notifications` table (pg 0042 / sqlite 0041) + indexes, registered in `migrate.rs` | `migrations/*`, `migrate.rs` |
| `Notification` / `NewNotification` models + `NotificationId` | `maidan-types/src/models.rs`, `ids.rs` |
| Store CRUD — `create_notification` / `list_notifications` / `mark_notification_read` / `mark_all_notifications_read` / `unread_notification_count`, both backends | `store.rs`, `store/{sqlite,postgres}/notifications.rs`, `store/*/mod.rs` |

## Design decisions

- **One row per (recipient, source event).** A `Notification` is *who* should know
  (`member_id`), *what* triggered it (`kind` + `source_log_id`), denormalized context
  (`channel/thread/message/actor`) so the inbox renders without re-fetching the
  event, and per-recipient read state (`read_at` NULL = unread). This is the
  per-recipient layer the shared mention row + single cursor can't express.
- **`kind` reuses `EventKind`.** A notification's kind *is* the triggering event's
  kind — no new vocabulary, no 11-site enum drill. Forward-compatible: any event kind
  can produce a notification. Stored as TEXT (`as_str`/`parse`), like the store's
  other EventKind columns.
- **`source_log_id` has no FK.** It points at the `maidan_events` row that triggered
  the notification, but deliberately *not* as a foreign key — notifications must
  survive event-log retention pruning (Cluster 186). The denormalized context is what
  keeps a notification renderable after its source event is gone.
- **`mark_read` is idempotent + existence-signalling.** `SET read_at =
  COALESCE(read_at, now())` preserves the first-read timestamp on a re-mark and
  returns whether the id exists — so the eventual REST route can 404 a bad id without
  a separate lookup, and a double mark-read is a harmless no-op.
- **Foundation only.** A new table + module; zero existing paths change (Clusters
  159 / 217 / 226 / 230 / 234 pattern).

## Non-goals / deferred (the rest of Program C)

- **Router** (Cluster 238) — a bus consumer that resolves an event to its recipients
  and writes rows (mentions first).
- **REST inbox** (239) + **MCP tools + `wait_for_notification`** (240).
- **Preferences / mute / follow** (Arc H, 241–243); **email/SMTP transport, digests,
  presence-aware routing, `/ui` notification center** (Arc I, 244–246).

## Risks

- None — a new table off every existing path. Migration registration is the standing
  gotcha (a `.sql` must get a `const` + `apply_*` call, both backends); covered by the
  both-backend store test, `dialect_parity`, and `concurrent_migrations`.
