# Cluster 212.0 retro — two events, one edit, and `message.rs` goes publish-free

> Tag **`v212.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 11.

## What shipped

- Message **edit** and **tombstone** migrated to the transactional-outbox pattern:
  `edit_message_with_event` (`MessageEdited`) and `tombstone_message_with_event`
  (`MessageTombstoned`) commit the mutation and its event in one tx. `message.rs`
  no longer calls `publish()` at all.

## Surprises / decisions

- **One edit, two events — factor the mutation, not the event.** Cluster 211's
  slash finalization edits a message and emits `MessagePosted`; a user edit does
  the identical mutation but emits `MessageEdited`. Rather than a second copy of
  the edit SQL, the mutation moved into a private `edit_in_tx(&mut tx, …) ->
  Message`, and the two public methods are each ~10 lines of "edit, resolve, build
  *their* event, append". Same shape as the FSM `transition_in_tx` split (208):
  when N callers share a mutation but differ in the event, the tx-core is the seam.
- **Tombstone's idempotence is already a guard, not a wrinkle.** Unlike
  remove_reaction / unpin (which return `bool`), `tombstone` already errors
  `NotFound` on a no-op (the `WHERE … tombstoned_at IS NULL` affects zero rows), so
  `tombstone_with_event` returns a plain `StoredEvent` — a re-tombstone fails before
  any event is appended. No `Option<StoredEvent>` needed.
- **A file empties out.** With post (211), edit, and tombstone all migrated,
  `message.rs` lost its last `publish` / `Event` / `Utc` — the whole import line
  went. A good marker of migration progress: the route file that touches messages
  most is now entirely on `publish_stored`.

## Capability table extension

| Change | Where |
|--------|-------|
| Message edit + tombstone transactional outbox (`edit_message_with_event`, `tombstone_message_with_event`; shared `edit_in_tx`) | `store/*/messages.rs` + `routes/message.rs` |

## Risks identified + still open

- **Mixed atomicity, short tail** (tracked) — `publish()` now serves only the
  **A2A ingest** post and the member / workspace / reference / artifact events,
  plus the federation **relay** (a re-publish of remote events, not a local write).

## Forward look

Next: the A2A ingest post (structurally the DM-post shape — a plain insert +
`MessagePosted`, so `post_message_with_event` fits) and the peripheral events
(member / workspace / reference / artifact). After those, `publish()`'s only caller
is the federation relay — rename it to its true role rather than delete. Program A
then finishes with federation ingest trust policy + an RLS spike, before Programs
B/C/D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 211.0]] transactional-outbox refactor.
