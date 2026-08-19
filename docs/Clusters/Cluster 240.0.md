# Cluster 240.0 — MCP inbox tools + `wait_for_notification` (closes Arc G)

> **Program C (notifications & reach), part 4** — **closes Arc G**. Phase XXIV
> post-gate hardening. Tag **`v240.0.0`**. No new gate tag.

## Goal

Give MCP-native agents the same notification surface REST got in 239, plus the
blocking wait that lets an agent *await* a notification. Closes the per-recipient
notification arc (ledger 237 → router 238 → REST 239 → MCP 240).

## Scope

| Change | Where |
|--------|-------|
| MCP `list_notifications` / `get_unread_count` / `mark_notification_read` — the twins of 239's REST, over the shared store | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `wait_for_notification` — block on the member's next notification-worthy event | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| Extracted `wait_for_member_event` helper shared by `wait_for_mention` + `wait_for_notification` | `tools/member.rs` |

## Design decisions

- **`wait_for_notification` is the general form of `wait_for_mention`.** Both block
  on a member-addressed event, RBAC-checked, returning the triggering event or
  `null`. `wait_for_mention` waits on `{MentionRecorded}`; `wait_for_notification`
  waits on `notifiable_kinds()` — the set the router (238) turns into notifications
  (today just mentions, so they currently coincide; Arc H broadens the notifiable
  set and `wait_for_notification` follows). Rather than duplicate the ~30-line poll
  loop, both delegate to a new `wait_for_member_event(server, auth, member_id, kinds,
  timeout_ms)`.
- **Return the event, not the ledger row.** The router writes the notification row
  asynchronously from a *separate* bus consumer, so fetching the row on the wait's
  event would race the router. Returning the triggering event (like
  `wait_for_ready`) is race-free; the durable ledger is for the *drain*
  (`list_notifications` / `get_unread_count`), the at-least-once path is `GET
  /mcp/stream`.
- **No channel-filter on `list_notifications`.** It mirrors the sibling inbox tools
  (`get_inbox` / `list_mentions`, Cluster 149) — a member-scoped read, no per-channel
  filter — rather than the aggregate `list_assigned_threads` model. `wait_for_notification`
  keeps the per-event `can_access_thread` guard (inherited from the shared helper),
  matching `wait_for_mention`.
- **Member-scoped args, no pre-dispatch gate.** All four key on `member_id` /
  `notification_id` (not a channel/thread), so they fall through the
  `enforce_channel_access` gate like the other inbox tools; `mark_notification_read`
  is recipient-scoped in the store (239), so it's safe even for a bearer.

## Non-goals

- No new store logic or event kind — REST (239) + MCP read/write the same
  `list/mark/unread_count` methods; `wait_for_notification` reuses the bus.

## Risks

- MCP 5-place wiring + both sorted contracts; the contract-sync tests
  (`tools_catalog_contract`, `mcp_capability_map_contract`) catch a miss. The
  `wait_for_mention` extraction is behaviour-preserving (its test still passes).
