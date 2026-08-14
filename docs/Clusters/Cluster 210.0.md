# Cluster 210.0 — transactional outbox: DM / group-DM posts

**Theme:** Program A (security & correctness round 2), part 9 — migrate the
DM and group-DM message-post paths to the transactional-outbox pattern.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v210.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `post_message_with_event(new, dm_conversation_id)` — insert message + `MessagePosted` in one tx | `store/{postgres,sqlite}/messages.rs`, `store.rs`, `*/mod.rs` |
| DM + group-DM post routes use it + `publish_stored` | `dm.rs`, `group_dm.rs` |

## Why

`MessagePosted` is the last message-scoped event still appended in a separate
transaction. The **regular** post path (`message.rs`) is entangled — it runs
slash-command processing that can *edit* the message after insert, so its event
must reflect the final message (that's the migration's hard last step). But the
**DM / group-DM** post paths do a plain insert with no post-insert edit, so they
can migrate cleanly now.

## The change

A new `post_message_with_event(new, dm_conversation_id)` store method inserts the
message and appends `MessagePosted` in one tx, resolving `(workspace, channel,
thread)` via the existing `message_scope_in_tx` and threading the caller-supplied
`dm_conversation_id` into the event (`Some` for a 1:1 DM, `None` for a group DM —
matching the pre-migration events exactly). Both post routes call it +
`publish_stored`, dropping their hand-built `MessagePosted` literal and the
now-redundant `resolve_thread_context` (the store resolves the scope).

## Exit criteria

- A DM/group-DM message and its `MessagePosted` event commit atomically —
  **met**.
- `v210.0.0` tagged.

## Verification & limits

- `event_log::dm_post_with_event_appends_atomically` (store): a 1:1-DM post
  carries `dm_conversation_id = Some(...)` in the durable event payload; a group
  post carries `None`; both events durable.
- Behaviour-preserving: `dm_e2e`, `group_dm_e2e`, `dm_participation_e2e`,
  `event_emission_e2e`, `ws_subscribe_e2e` + the store suite (both backends) green.
- **Limit (tracked):** only the slash-edit-entangled **regular** message-post path
  still uses `publish()`. That's the last mutation; once it migrates, `publish()`
  is deleted and the outbox refactor is complete.

## References

- [[Retros/Cluster 210.0]]; `store/*/messages.rs`, `dm.rs`, `group_dm.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 209.0]].
