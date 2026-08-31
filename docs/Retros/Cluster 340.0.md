# Cluster 340.0 retro — fetch-once message authorization (audit P1.4c)

> Tag **`v340.0.0`**. Phase XXIV (post-gate hardening). **Cluster 9 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The message-keyed twin of Cluster 339, completing the post-path round-trip-reduction theme (audit
P1.4). ~12 handlers in `message.rs`/`social.rs` called `resolve_message_chain` (get_message +
thread + channel) and then `ensure_thread_access`/`ensure_message_access` (thread + channel
again), plus a redundant `ensure_workspace`.

- **`maidan_auth::authorize_message(store, auth, message_id) -> MessageScope`** — one
  message→thread→channel fetch resolves `{workspace_id, channel_id, thread_id, message_id}` AND
  authorizes the caller (delegating to `authorize_thread`); bypass skips the checks but still
  returns the scope. `ensure_message_access` now delegates to it (rule single-sourced).
- **Handlers migrated**: those that use the scope (`edit_message`, `tombstone_message`,
  `purge_message`, `seed_from_message`) call `authorize_message`; those that only need the check
  (votes, reactions, `get_message`, message-edits, `create_mention`) drop `resolve_message_chain`
  + `ensure_workspace` and keep `ensure_message_access`. Per-request message-scoped fetches drop
  from ~5 to 3.

## Surprises / decisions

- **`MessageScope` mirrors the router's `MessageChain`.** Same field names, so a handler that kept
  a `chain` from `resolve_message_chain` reads it unchanged — the migration is a preamble swap.
- **`ensure_message_access` was already 3 fetches; the waste was the caller's extra `resolve`.**
  The double-fetch lived in handlers that resolved the chain *and then* called an access helper
  that resolved it again; `authorize_message` collapses that to the single resolve+authorize the
  helper was already doing.
- **Layered on 339.** `authorize_message` = `get_message` + `authorize_thread`, so the DM /
  private-channel rule and the 404/403 mapping are inherited unchanged — no new authorization
  logic, only a new entry point that returns the scope.

## Test evidence

`channel_access_e2e` (6), `dm_participation_e2e`, `http_capability_matrix_e2e`,
`event_emission_e2e`, `post_mention_routing_e2e`, `reactions_pins_e2e`, `seed_from_message_e2e`,
`vote_confidence_e2e`, `message_content_e2e`, `mention_webhook_e2e`, `ui_edit_history_e2e` — all
green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Audit P1.4 (post-path round-trip reduction) is now complete (338 mentions, 339 thread-keyed, 340
message-keyed). The channel-keyed `resolve_channel_context` sites (create/list threads) are a
small residual variant, left as-is (only 2 handlers, low traffic). Next: **P1.5** egress
wire-path tests + LSN replica CI → **P2** docs/polish (gRPC doc contradiction, tool-count drift,
Integration.md flagship-surface gaps).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
