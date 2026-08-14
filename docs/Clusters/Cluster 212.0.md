# Cluster 212.0 — transactional outbox: message edit + tombstone

**Theme:** Program A (security & correctness round 2), part 11 — migrate the
message **edit** and **tombstone** mutations to the transactional-outbox pattern.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v212.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `edit_in_tx` core extracted; `edit_message_with_event` (`MessageEdited`) | `store/{postgres,sqlite}/messages.rs`, `store.rs`, `*/mod.rs` |
| `tombstone_message_with_event` (`MessageTombstoned`) | `store/{postgres,sqlite}/messages.rs`, `store.rs`, `*/mod.rs` |
| `edit_message` + `tombstone_message` routes use them + `publish_stored` | `routes/message.rs` |

## Why

After the post paths (Clusters 210/211), `MessageEdited` and `MessageTombstoned`
were the next message-scoped events still appended in a separate transaction.
Migrating them makes an edit / tombstone crash-consistent with its event, and
empties `message.rs` of `publish()` calls entirely.

## The change

- **Shared edit core.** Cluster 211's `edit_with_posted_event` (emits
  `MessagePosted`) and the new `edit_with_event` (emits `MessageEdited`) differ
  only in the event, so the edit SQL (history row on body change + UPDATE) is
  extracted into a private `edit_in_tx(&mut tx, …) -> Message` both call.
- **`tombstone_with_event`** does the tombstone UPDATE + `MessageTombstoned` in one
  tx (`NotFound` if already tombstoned — the same guard the plain `tombstone` has,
  so no event on a no-op).
- Both events carry `dm_conversation_id` (resolved by the route via
  `dm_conversation_id_for_thread`, passed as a parameter — same shape as the post
  paths). The routes call `*_with_event` + `publish_stored`; `message.rs` loses its
  last `publish` / `Event` / `Utc` references.

## Exit criteria

- An edit / tombstone and its event commit atomically — **met**.
- `v212.0.0` tagged.

## Verification & limits

- `event_log::edit_and_tombstone_with_event_append_atomically` (store): an edit
  emits a durable `MessageEdited` (+ one history row on the body change); a
  tombstone emits `MessageTombstoned`; a re-tombstone is `NotFound` (no event).
- Behaviour-preserving: `http_crud_e2e`, `message_content_e2e`,
  `ui_edit_history_e2e`, `event_emission_e2e`, `outbox_http_e2e` + the store suite
  (both backends) green.
- **Limit (tracked):** `publish()` now serves only the **A2A ingest** post and the
  member / workspace / reference / artifact events (+ the federation relay). Those
  are the last migration targets before `publish()`'s only caller is the relay.

## References

- [[Retros/Cluster 212.0]]; `store/*/messages.rs`, `routes/message.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 211.0]].
