# Cluster 338.0 retro — post-path mention-routing round-trip reduction (audit P1.4a)

> Tag **`v338.0.0`**. Phase XXIV (post-gate hardening). **Cluster 7 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The first half of audit P1.4 (post-path round-trip reduction). Every message post — the hottest
write path — paid a redundant `resolve_message_chain` (message → thread → channel → workspace)
purely to re-derive a workspace id the caller already had, and did so even when the body had no
`@handles` at all.

- **`publish_routed_mentions`** (REST `routes/mod.rs` + MCP `tools/message.rs`) now:
  - short-circuits on `parse_at_handles(&body).is_empty()` — the common case — so a post with no
    mentions does **zero** store work;
  - otherwise calls `route_mentions_in_message(store, workspace_id, …)` directly with the
    workspace the caller already resolved, dropping the per-post `resolve_message_chain`.
- Removed the now-unused `route_mentions_for_message` from `maidan-router` (its only two callers
  were the two `publish_routed_mentions`).

## Surprises / decisions

- **The workspace id was always in hand.** Both `publish_routed_mentions` signatures already take
  `workspace_id` (the post handler resolved it for the `MessagePosted` event); the mention helper
  threw it away and re-fetched. The fix is subtraction, not new plumbing.
- **Short-circuit before the async call, not inside it.** `route_mentions_in_message` already
  no-ops on zero handles, but only after being entered; parsing handles is a cheap pure string
  scan, so gating on it in the caller keeps the whole mention subsystem off the no-mention path.
- **Behaviour-preserving.** Mentions still route and emit `MentionRecorded` identically; only the
  query count changed. The two existing end-to-end mention tests are the regression guard.

## Test evidence

New `post_mention_routing_e2e::post_message_routes_at_handles_and_skips_plain_posts` (subscribes
to the bus, asserts an `@handle` post emits exactly one `MentionRecorded` for the target and a
plain post emits `MessagePosted` and none). Existing `mention_webhook_e2e` +
`notification_router_e2e` + the full `maidan-mcp` lib suite (61) green. fmt + strict clippy +
`--all-targets` + bootstrap-strip clean.

## Forward look

**339 (P1.4b):** the systemic thread+channel double-fetch — ~30 handlers call
`resolve_thread_context` (get_thread + get_channel) then `ensure_thread_access` (the same two
fetches again). Plan: a fetch-once `authorize_thread` helper in `maidan-auth` that returns the
resolved scope and performs the access check, with `ensure_thread_access` delegating to it
(behaviour-identical), then migrate the double-fetch pairs. Then P1.5 (egress wire tests + LSN
replica CI) → P2 (docs/polish).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
