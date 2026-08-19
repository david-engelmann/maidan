# Cluster 240.0 retro — agents await notifications (Arc G closes)

> Tag **`v240.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 4** — **closes Arc G**.

## What shipped

- MCP `list_notifications` / `get_unread_count` / `mark_notification_read` — the twins
  of 239's REST, over the shared store.
- MCP `wait_for_notification` — block on the member's next notification-worthy event.
- A shared `wait_for_member_event` helper backing both `wait_for_mention` and
  `wait_for_notification`.

An MCP-native agent can now drain its inbox (`list`/`count`), clear it (`mark`), and
*await* new notifications — the whole per-recipient arc (ledger → router → REST → MCP)
is complete.

## Surprises / decisions

- **`wait_for_notification` is `wait_for_mention` generalized — so share the loop.**
  The two differ only in which event kinds they wait on (`{MentionRecorded}` vs the
  router's `notifiable_kinds()`, currently the same set). Copying the ~30-line
  subscribe/poll/RBAC loop into a second tool would be exactly the kind of
  duplication that drifts; extracting `wait_for_member_event` and delegating from both
  keeps one implementation. The extraction is behaviour-preserving — the existing
  `wait_for_mention` test passed unchanged.
- **Return the event, not the row — because the router is a different consumer.** The
  tempting thing is for `wait_for_notification` to return the actual `Notification`
  row, but the router writes that row from a *separate* bus consumer, so the wait's
  own subscription can fire before the row lands. Returning the triggering event (the
  `wait_for_ready` model) is race-free; the ledger backs the drain, not the wait's
  return value. Honest about the overlap with `wait_for_mention` today — it's the
  forward-looking surface, and the doc says so.
- **Match the sibling inbox tools, not the aggregate model.** `list_notifications`
  could RBAC-filter by `can_access_thread` like `list_assigned_threads`, but the
  closer precedent is `get_inbox` / `list_mentions` — member-scoped reads with no
  per-channel filter — and notifications *are* the inbox generalization. So no filter
  on the bulk read; `wait_for_notification` keeps the per-event access guard (via the
  shared helper), exactly as `wait_for_mention` does. Consistent with the pair it
  extends.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `list_notifications` / `get_unread_count` / `mark_notification_read` / `wait_for_notification` + shared `wait_for_member_event` | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None new. The notification surface is complete over REST (239) + MCP (240).

## Forward look

**Arc G (per-recipient notification ledger + router + unified inbox) is complete**
(237 ledger → 238 router → 239 REST → 240 MCP). Next: **Arc H — preferences +
subscription** (mute / per-kind prefs / follow a thread or channel; `route_event`
grows its notifiable set and consults prefs), then **Arc I** (email/SMTP transport,
digests, presence-aware routing, `/ui` notification center). Then **Program D (scale &
durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 239.0]].
