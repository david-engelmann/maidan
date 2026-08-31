# Cluster 339.0 retro — fetch-once thread authorization (audit P1.4b)

> Tag **`v339.0.0`**. Phase XXIV (post-gate hardening). **Cluster 8 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The second half of audit P1.4 (post-path round-trip reduction) — the systemic thread+channel
double-fetch. ~30 handlers across `message.rs`/`thread.rs`/`social.rs`/`skills.rs` called
`resolve_thread_context` (get_thread + get_channel) and then `ensure_thread_access` (the same two
fetches again), plus a redundant explicit `ensure_workspace` the access check already performs.

- **`maidan_auth::authorize_thread(store, auth, thread_id) -> ThreadScope`** — one fetch resolves
  `{workspace_id, channel_id, thread_id}` AND authorizes the caller (identical rule: workspace
  isolation, then DM-participation for `__dm__` or `channel_members` for a private channel; bypass
  skips the checks but still returns the scope, which the handler needs regardless).
- **`ensure_thread_access` now delegates** to `authorize_thread` (preserving its bypass
  early-return), so the access rule is single-sourced — and it also sheds the duplicate
  `get_channel` that its old `ensure_channel_access` tail re-did.
- **Handlers migrated**: those that use the resolved scope call `authorize_thread`; those that only
  needed the check drop `resolve_thread_context` + `ensure_workspace` and keep `ensure_thread_access`.
  Either way the per-request thread+channel fetch count on the thread-scoped surface halves.

## Surprises / decisions

- **Behaviour-identical, provably.** A missing thread → 404 (`StoreError::NotFound` maps the same
  through `AuthError` as it did through `RouterError`); wrong workspace / no access → 403 with the
  same `Forbidden` messages. Verified end to end by the access-control e2e suite.
- **`ensure_thread_access` kept, not replaced, where the scope is unused.** For a read that only
  needs the check, `ensure_thread_access` (which now *is* `authorize_thread` minus the return) is
  the clearer call and preserves the bypass-no-fetch optimization; `authorize_thread` is used only
  where the handler consumes `workspace_id`/`channel_id`.
- **Capability-before-existence precedence.** A few read handlers checked `resolve` before `cap`;
  the migration puts `cap` first (a cap-less caller now gets 403 before existence is probed — a
  cleaner, slightly more private ordering). The capability-matrix test uses a real thread id, so
  it is unaffected.
- **Deferred to Cluster 340:** the *message*-keyed twin — `resolve_message_chain` (get_message +
  thread + channel) then `ensure_thread_access`/`ensure_message_access` on edit / tombstone /
  votes / reactions — has the same double-fetch shape and wants an `authorize_message` helper.
  The channel-keyed `resolve_channel_context` sites (create/list threads) are a third, smaller
  variant.

## Test evidence

Full access-control suite green — `channel_access_e2e` (6), `dm_participation_e2e`,
`thread_dependencies_e2e`, `thread_result_e2e`, `http_capability_matrix_e2e` — plus the migrated
routes: `assignment_readside`, `context_snapshot`, `dm_e2e`, `event_emission`, `glossary_context`,
`group_dm`, `post_mention_routing`, `skills_rest`, `thread_assignment`, `thread_ready`,
`tool_transcript`. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

**340 (P1.4c, optional):** `authorize_message` for the message-keyed double-fetch. Then P1.5
(egress wire tests + LSN replica CI) → P2 (docs/polish).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
