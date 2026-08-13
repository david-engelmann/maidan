# Cluster 206.0 — transactional outbox: votes + reactions

**Theme:** Program A (security & correctness round 2), part 5 — continue the
transactional-outbox migration (Cluster 205's pattern) to the vote + reaction
mutations.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v206.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `events::message_scope_in_tx` — resolve a message's (ws, channel, thread) in-tx | `store/{postgres,sqlite}/events.rs` |
| `cast_vote_with_event`, `add_reaction_with_event`, `remove_reaction_with_event` | `store/{postgres,sqlite}/{votes,reactions}.rs`, `store.rs`, `*/mod.rs` |
| Routes use `*_with_event` + `publish_stored` | `routes/social.rs` |

## Why

Cluster 205 established the transactional-outbox pattern (domain row + event in
one tx) on channel/thread create. This cluster migrates the next batch — votes and
reactions — so their `VoteCast` / `ReactionAdded` / `ReactionRemoved` events are
crash-consistent with the mutation, not appended in a separate transaction.

## The change

Each mutation gains a `*_with_event` variant that opens a tx, does the row
mutation, resolves the message's `(workspace, channel, thread)` **in the same tx**
via a new shared `events::message_scope_in_tx` (the message-scoped events need
workspace + thread; the resolver also returns channel for pins next cluster),
builds the event, `append_in_tx`, and commits. The route calls it + `publish_stored`.

**The one wrinkle — conditional events.** `remove_reaction` is idempotent: it
returns whether a row was actually deleted. So `remove_reaction_with_event`
returns `(removed, Option<StoredEvent>)` and appends the event **only** when a row
was removed — a no-op removal produces no event (matching the old route's `if
removed { publish }`). The route does `if let Some(stored) = stored {
publish_stored }`.

## Exit criteria

- Vote/reaction rows and their events commit atomically; a no-op reaction removal
  produces no event — **met**.
- `v206.0.0` tagged.

## Verification & limits

- `event_log::social_with_event_appends_atomically` (store): a cast vote + added
  reaction produce durable `VoteCast`/`ReactionAdded` events; a missed removal →
  `(false, None)`, a real one → `(true, Some)`.
- Behaviour-preserving: `reactions_pins_e2e`, `event_emission_e2e`, `mcp_e2e`,
  `ws_subscribe_e2e`, `ui_collab_e2e` + the store suite (both backends) green.
- **Limit (tracked):** the migration continues — pins + mentions, thread
  transitions/assignments, DM/group-DM posts, and the slash-edit-entangled
  message post still use the retry-hardened `publish()`. When all are migrated,
  `publish()` is deleted.

## References

- [[Retros/Cluster 206.0]]; `store/*/events.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Program A). Continues [[Retros/Cluster 205.0]].
