# Cluster 206.0 retro — the pattern scales, and idempotent removals need care

> Tag **`v206.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 5.

## What shipped

- Votes + reactions migrated to the transactional-outbox pattern: `*_with_event`
  variants that commit the row and its event in one tx, over a shared
  `message_scope_in_tx` resolver. `ReactionRemoved` is conditional — only when a
  row was actually removed.

## Surprises / decisions

- **The resolver is the reuse.** Every message-scoped event needs the same
  `(workspace, channel, thread)` context resolved from a `message_id`. Extracting
  `message_scope_in_tx` once (per backend) means each `*_with_event` is ~10 lines
  of "mutate, resolve, build event, append" — the migration is now
  copy-the-shape, not re-derive-the-context. Returning channel too (even though
  votes/reactions don't need it) sets up pins next cluster for free.
- **Idempotent mutations break the "always append" shape.** `remove_reaction`
  returns whether anything was deleted, and the old route only published on a real
  removal. A naive `remove_reaction_with_event -> StoredEvent` would fabricate a
  `ReactionRemoved` for a no-op — a phantom event. So the signature is
  `(bool, Option<StoredEvent>)`: append inside the tx *only* when
  `rows_affected > 0`. Every conditional mutation (unpin next) needs this shape;
  it's the wrinkle to watch for the rest of the migration.
- **Behaviour-preserving is still the bar.** The same events must reach
  subscribers; `event_emission_e2e` staying green proves votes/reactions still
  emit through `publish_stored`, now atomically.

## Decisions

- **Batch by resolver, not by count.** Votes + reactions share `message_scope_in_tx`
  and are non-conditional (except remove), so they're a coherent batch. Pins +
  mentions (also message-scoped, pins conditional) are the next batch; transitions
  / assignments (thread-scoped, different resolution) are their own.
- **Keep `publish()` until it has no callers.** The migration is deliberately
  incremental-toward-complete; non-migrated mutations keep the retry-hardened
  path, so there's never a regression, just a shrinking non-atomic set.

## Capability table extension

| Change | Where |
|--------|-------|
| Vote + reaction transactional outbox (`*_with_event` + `message_scope_in_tx`) | `store/*/{votes,reactions,events}.rs` + `routes/social.rs` |

## Risks identified + still open

- **Mixed atomicity, shrinking** (tracked) — pins/mentions/transitions/assignments/
  DM posts/message-post still use `publish()`. The message-post path remains the
  hard one (build the event after the slash-command edit).

## Forward look

Migration continues: pins + mentions (next), then thread transitions + assignments
+ DM/group-DM posts, then the entangled message-post path, then delete `publish()`.
Program A also has federation ingest trust policy + an RLS spike. Then Programs B
(agentic orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 205.0]] transactional-outbox refactor.
