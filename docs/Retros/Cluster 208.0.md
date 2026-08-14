# Cluster 208.0 retro — a new resolver, and refactor-don't-duplicate

> Tag **`v208.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 7.

## What shipped

- Thread FSM transitions migrated to the transactional-outbox pattern:
  `transition_thread_with_event` commits the state change **and** its
  `ThreadStateChanged` event in one tx, over a new `events::thread_scope_in_tx`
  resolver (the thread-scoped twin of `message_scope_in_tx`).

## Surprises / decisions

- **The batch changed resolvers, so it needed a new one.** 206/207's
  `message_scope_in_tx` resolves `(ws, channel, thread)` from a `message_id`;
  transitions/assignments have only a `thread_id`, so this cluster adds
  `thread_scope_in_tx(&mut tx, thread_id) → (ws, channel)`. Introducing it here
  (with the first thread-scoped mutation) sets up Cluster 209's assignments for
  free — the exact 205→206/207 rhythm (seed the resolver, then reuse it).
- **Refactor the in-tx core, don't duplicate it.** Unlike votes/pins (short SQL),
  `transition` is a ~75-line FSM step (read → validate → HSM parent check →
  insert transition row → update state), and it was already fully transactional.
  Copy-pasting it into `transition_with_event` would have forked the FSM logic.
  Instead the body moved into a private `transition_in_tx(&mut tx, …)` that stops
  short of the commit; `transition` (commit only) and `transition_with_event`
  (append event, then commit) are each ~6 lines on top of it. One copy of the
  logic, two commit policies.
- **Split the thread-scoped batch.** Transitions (one mutation, `ThreadStateChanged`)
  and assignments (four mutations, `ThreadAssignmentChanged`, two conditional) are
  both thread-scoped but distinct events; shipping transitions alone keeps this
  cluster focused and lets the resolver land + bake before 209 leans on it.

## Capability table extension

| Change | Where |
|--------|-------|
| Thread transition transactional outbox (`transition_thread_with_event` + `thread_scope_in_tx`) | `store/*/{thread_transitions,events}.rs` + `routes/thread.rs` |

## Risks identified + still open

- **Mixed atomicity, shrinking** (tracked) — assignments, DM/group-DM posts, and
  message-post still use `publish()`. The message-post path remains the hard one
  (build the event after the slash-command edit).

## Forward look

Cluster 209 migrates the assignment mutations (assign/unassign/claim/claim_next →
`ThreadAssignmentChanged`) reusing `thread_scope_in_tx` — two of them conditional
(claim / claim_next only emit when they actually claimed). Then DM/group-DM posts,
then the entangled message-post path, then delete `publish()`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 207.0]] transactional-outbox refactor.
