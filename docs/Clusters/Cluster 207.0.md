# Cluster 207.0 — transactional outbox: pins + mentions

**Theme:** Program A (security & correctness round 2), part 6 — continue the
transactional-outbox migration (Cluster 205's pattern) to the pin + mention
mutations.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v207.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `pin_message_with_event`, `unpin_message_with_event` (conditional) | `store/{postgres,sqlite}/pins.rs`, `store.rs`, `*/mod.rs` |
| `record_mention_with_event` | `store/{postgres,sqlite}/mentions.rs`, `store.rs`, `*/mod.rs` |
| Routes use `*_with_event` + `publish_stored` | `routes/social.rs` (pin/unpin), `routes/message.rs` (mention) |

## Why

Cluster 206 migrated votes + reactions. This cluster migrates the next
message-scoped batch — pins and mentions — so `MessagePinned` /
`MessageUnpinned` / `MentionRecorded` are crash-consistent with the row, over
the same shared `events::message_scope_in_tx` resolver (pins need the channel
too; mentions discard it).

## The change

Each mutation gains a `*_with_event` variant that opens a tx, does the row
mutation, resolves the message's `(workspace, channel, thread)` **in the same
tx**, builds the event, `append_in_tx`, and commits. The route calls it +
`publish_stored`.

**The conditional wrinkle (again).** `unpin_message` is idempotent, so
`unpin_message_with_event` returns `(removed, Option<StoredEvent>)` and appends
`MessageUnpinned` **only** when a row was removed — matching the old route's `if
unpinned { publish }`. `pin` and `record_mention` are unconditional (`ON
CONFLICT DO NOTHING` on a re-pin/re-mention still returns a `StoredEvent`,
preserving the old always-publish behaviour).

## Exit criteria

- Pin/mention rows and their events commit atomically; a no-op unpin produces no
  event — **met**.
- `v207.0.0` tagged.

## Verification & limits

- `event_log::pins_and_mentions_with_event_append_atomically` (store): a pin +
  mention produce durable `MessagePinned`/`MentionRecorded`; an unpin miss →
  `(false, None)`, a real one → `(true, Some)` with `MessageUnpinned`.
- Behaviour-preserving: `reactions_pins_e2e`, `mentions_e2e`,
  `event_emission_e2e`, `ws_subscribe_e2e` + the store suite (both backends)
  green.
- **Limit (tracked):** the migration continues — thread transitions/assignments,
  DM/group-DM posts, and the slash-edit-entangled message post still use the
  retry-hardened `publish()`. When all are migrated, `publish()` is deleted.

## References

- [[Retros/Cluster 207.0]]; `store/*/{pins,mentions,events}.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 206.0]].
