# Cluster 207.0 retro — copy-the-shape, and a channel resolved for free

> Tag **`v207.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 6.

## What shipped

- Pins + mentions migrated to the transactional-outbox pattern:
  `pin_message_with_event`, `unpin_message_with_event` (conditional), and
  `record_mention_with_event` — each commits the row and its event in one tx over
  the shared `message_scope_in_tx` resolver.

## Surprises / decisions

- **The resolver paid off exactly as planned.** Cluster 206 returned `channel`
  from `message_scope_in_tx` even though votes/reactions didn't need it,
  specifically so pins would get it for free. This cluster cashed that in:
  `MessagePinned`/`MessageUnpinned` need `channel_id`, and the resolver already
  had it — the migration was pure copy-the-shape, no new SQL.
- **Conditional, once more.** `unpin` is the only idempotent mutation in the
  batch, so it's the only `(bool, Option<StoredEvent>)` — same wrinkle as
  `remove_reaction`. Pin and mention are unconditional: an `ON CONFLICT DO
  NOTHING` re-pin still returns a `StoredEvent`, which preserves the old route's
  always-publish behaviour (the pre-migration route published on every pin call,
  conflict or not).
- **The route shrank.** Migrating a handler *removes* code: the `publish(Event::…
  { occurred_at, workspace_id, channel_id, … })` literal (7+ lines) collapses to
  `publish_stored(&state, stored)`. `social.rs` lost its last `Event::`/`Utc::now`
  references — the whole import line went with them.

## Decisions

- **Batch by resolver, still.** Pins + mentions are message-scoped (same
  resolver), so they're one coherent batch. Thread transitions + assignments are
  thread-scoped (different resolution) and are the next batch; DM/group-DM posts
  and the entangled message-post path follow.
- **Keep `publish()` until it has no callers.** Unchanged from 206 — the
  non-migrated mutations keep the retry-hardened path, so the non-atomic set just
  shrinks.

## Capability table extension

| Change | Where |
|--------|-------|
| Pin + mention transactional outbox (`*_with_event`) | `store/*/{pins,mentions,events}.rs` + `routes/{social,message}.rs` |

## Risks identified + still open

- **Mixed atomicity, shrinking** (tracked) — thread transitions/assignments,
  DM/group-DM posts, and message-post still use `publish()`. The message-post path
  remains the hard one (build the event after the slash-command edit).

## Forward look

Migration continues: thread transitions + assignments (thread-scoped resolver),
then DM/group-DM posts, then the entangled message-post path, then delete
`publish()`. Program A also has federation ingest trust policy + an RLS spike.
Then Programs B (agentic orchestration), C (notifications & reach), D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 206.0]] transactional-outbox refactor.
