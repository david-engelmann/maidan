# Cluster 209.0 — transactional outbox: thread assignments

**Theme:** Program A (security & correctness round 2), part 8 — finish the
thread-scoped batch by migrating the assignment mutations to the
transactional-outbox pattern.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v209.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `assign_thread_with_event`, `unassign_thread_with_event` (capture previous in-tx) | `store/{postgres,sqlite}/threads.rs`, `store.rs`, `*/mod.rs` |
| `claim_thread_with_event`, `claim_next_thread_with_event` (conditional) | `store/{postgres,sqlite}/threads.rs`, `store.rs`, `*/mod.rs` |
| Routes use `*_with_event` + `publish_stored`; `publish_assignment` helper removed | `routes/thread.rs` |

## Why

Cluster 208 introduced the `thread_scope_in_tx` resolver and migrated the FSM
transition. This cluster reuses that resolver for the four assignment mutations,
so every `ThreadAssignmentChanged` commits atomically with its assignee change.

## The change

Each mutation gains a `*_with_event` store variant that opens a tx, does the
assignee UPDATE, resolves `(workspace, channel)` via `thread_scope_in_tx`, builds
`ThreadAssignmentChanged`, `append_in_tx`, and commits — with a shared
`append_assignment_event` helper (per backend) doing the resolve-build-append.

- **assign / unassign:** the previous assignee is now read **inside the tx** (a
  `SELECT assignee_id` before the UPDATE), replacing the route's separate
  `get_thread` — a consistent read, no race window. assign carries the handoff
  `note`; unassign carries none.
- **claim / claim_next — conditional.** These return `(result, Option<StoredEvent>)`
  / `(Option<Thread>, Option<StoredEvent>)`: the event is appended **only** when
  the CAS actually claimed (an already-assigned thread / empty channel emits
  nothing), matching the old route's `if claimed { publish }`.
- The route's `publish_assignment` helper is deleted (its hand-built event literal
  now lives in the store); `renew_claim` is untouched (it emits no event).

## Exit criteria

- Every assignment change and its `ThreadAssignmentChanged` event commit
  atomically; a no-op claim produces no event — **met**.
- `v209.0.0` tagged.

## Verification & limits

- `event_log::assignment_with_event_appends_atomically` (store): assign (previous
  `None`, note carried) → unassign → claim (event) → claim-again (no event) →
  claim_next (event) → claim_next-empty (no event); all emitted events durable.
- Behaviour-preserving: `thread_assignment_e2e`, `assignment_readside_e2e`,
  `event_emission_e2e` + the store suite (both backends) green.
- **Preserved quirk:** `claim_next`'s event still reports `previous_assignee_id =
  None` even on a lease-expiry reclaim (matches the pre-migration route; surfacing
  the reclaimed holder is a deliberate non-goal here).
- **Limit (tracked):** DM/group-DM posts and the slash-edit-entangled message post
  still use `publish()`. With assignments migrated, the whole thread-scoped batch
  is done.

## References

- [[Retros/Cluster 209.0]]; `store/*/threads.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Program A). Continues [[Retros/Cluster 208.0]].
