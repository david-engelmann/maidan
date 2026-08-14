# Cluster 210.0 retro — the easy MessagePosted first, before the hard one

> Tag **`v210.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 9.

## What shipped

- The DM and group-DM post paths migrated to the transactional-outbox pattern via
  a new `post_message_with_event(new, dm_conversation_id)` — insert + `MessagePosted`
  in one tx, over the existing `message_scope_in_tx` resolver.

## Surprises / decisions

- **`MessagePosted` splits into an easy path and a hard path.** The same event
  kind is emitted by three routes: DM, group-DM, and the regular message post.
  The regular one runs slash-command processing that *edits* the message after
  insert, so its event has to reflect the post-edit message — you can't build it
  at insert time. The two DM paths do a plain insert with no post-edit, so they
  migrate cleanly. Splitting `MessagePosted` this way lets 210 take the tractable
  two-thirds now and leaves only the genuinely-entangled path for last.
- **`dm_conversation_id` is a parameter, not a lookup.** The store's message
  insert knows nothing about DM conversations; the 1:1 vs group distinction lives
  in the route. So the method takes `dm_conversation_id: Option<DmConversationId>`
  and threads it straight into the event — `Some(dm.id)` from the DM route, `None`
  from the group route — reproducing the pre-migration events byte-for-byte.
- **Migrating removed a query, not just added atomicity.** Each route dropped its
  `resolve_thread_context` call (used only to fill the event's `channel_id`); the
  store now resolves the whole scope in-tx, so the routes lost a round-trip *and* a
  hand-built event literal.

## Capability table extension

| Change | Where |
|--------|-------|
| DM/group-DM post transactional outbox (`post_message_with_event`) | `store/*/messages.rs` + `dm.rs`, `group_dm.rs` |

## Risks identified + still open

- **Mixed atomicity, nearly gone** (tracked) — only the slash-edit-entangled
  regular message-post path still uses `publish()`. It's the last one; migrating
  it (build the event in-store after the slash edit) closes the refactor and
  deletes `publish()`.

## Forward look

One mutation left: the regular `post_message` route. It inserts, then may run a
slash-command edit, then publishes — so the `*_with_event` needs to build the
event *after* the edit, in-store. Once done, `publish()` is deleted and the
transactional-outbox refactor (205–211) is complete. Program A then finishes with
federation ingest trust policy + an RLS spike, before Programs B/C/D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 209.0]] transactional-outbox refactor.
