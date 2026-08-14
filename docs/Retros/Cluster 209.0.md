# Cluster 209.0 retro — the thread-scoped batch closes, and a read moves in-tx

> Tag **`v209.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 8.

## What shipped

- The four assignment mutations (assign / unassign / claim / claim_next) migrated
  to the transactional-outbox pattern, reusing Cluster 208's `thread_scope_in_tx`.
  The route's `publish_assignment` helper is gone.

## Surprises / decisions

- **The migration fixed a latent race.** The old `assign`/`unassign` route read
  the previous assignee with a *separate* `get_thread` before the mutation — a
  read-then-write window where a concurrent assignment could make the event's
  `previous_assignee_id` wrong. Moving that `SELECT assignee_id` into the same tx
  as the UPDATE makes the previous/new pair a consistent snapshot. Atomicity
  wasn't the only win here; the outbox tx also closed a correctness gap.
- **Two conditionals in one batch.** Both claim and claim_next are CAS mutations,
  so both are `(…, Option<StoredEvent>)` — the event rides only a real claim. It's
  the same wrinkle as remove_reaction / unpin, now the majority of the batch.
- **A shared per-backend helper earned its keep.** All four events are the same
  `ThreadAssignmentChanged` shape, differing only in previous/note, so an
  `append_assignment_event(tx, thread, actor, previous, note)` collapses the
  resolve-build-append into one call site each — four mutations, one event
  builder.
- **Preserved a quirk on purpose.** `claim_next` can reclaim a lease-expired
  thread that had a prior holder, but the pre-migration route reported
  `previous_assignee_id = None` there. Behaviour-preserving is the migration's
  bar, so the `*_with_event` variant keeps `None` — surfacing the reclaimed holder
  is a separate, deliberate non-goal (noted, not done).

## Capability table extension

| Change | Where |
|--------|-------|
| Thread assignment transactional outbox (`assign/unassign/claim/claim_next_with_event`) | `store/*/threads.rs` + `routes/thread.rs` |

## Risks identified + still open

- **Mixed atomicity, shrinking** (tracked) — only DM/group-DM posts and the
  slash-edit-entangled message post still use `publish()`. The message-post path
  remains the hard one (build the event after the slash-command edit).

## Forward look

The thread-scoped batch is complete. Remaining outbox work: DM/group-DM posts,
then the entangled message-post path, then delete `publish()`. Program A also has
federation ingest trust policy + an RLS spike. Then Programs B (agentic
orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 208.0]] transactional-outbox refactor.
