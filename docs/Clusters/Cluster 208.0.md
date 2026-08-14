# Cluster 208.0 — transactional outbox: thread transitions

**Theme:** Program A (security & correctness round 2), part 7 — continue the
transactional-outbox migration to the thread-scoped mutations, starting with FSM
state transitions.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v208.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `events::thread_scope_in_tx` — resolve a thread's (ws, channel) in-tx | `store/{postgres,sqlite}/events.rs` |
| `transition_thread_with_event` (+ extracted `transition_in_tx` core) | `store/{postgres,sqlite}/thread_transitions.rs`, `store.rs`, `*/mod.rs` |
| Route uses `*_with_event` + `publish_stored` | `routes/thread.rs` |

## Why

Clusters 205–207 migrated the message-scoped mutations (channel/thread create,
votes, reactions, pins, mentions). The next batch is **thread-scoped**: the FSM
transition and the assignment mutations resolve their event context from a
`thread_id`, not a `message_id`. This cluster introduces the shared
`thread_scope_in_tx` resolver (the thread-scoped twin of `message_scope_in_tx`)
and migrates the FSM transition; the assignment mutations follow in Cluster 209,
reusing the resolver — the same 205→206/207 shape.

## The change

The existing `transition` already ran the whole FSM step (read → validate →
insert transition row → update state) in a transaction. It's refactored into a
private `transition_in_tx(&mut tx, …)` core (everything except the commit), so
both `transition` (commit only) and the new `transition_with_event` (resolve
`(workspace, channel)` via `thread_scope_in_tx`, build `ThreadStateChanged`,
`append_in_tx`, commit) share one copy of the FSM logic — no duplication. The
route drops its hand-built `ThreadStateChanged` literal for `*_with_event` +
`publish_stored`.

## Exit criteria

- A thread transition and its `ThreadStateChanged` event commit atomically —
  **met**.
- `v208.0.0` tagged.

## Verification & limits

- `event_log::transition_with_event_appends_atomically` (store): an open→in_review
  transition commits the state change **and** a durable `ThreadStateChanged`.
- Behaviour-preserving: `thread_transition_e2e`, `event_emission_e2e`,
  `fsm_hooks_e2e`, `thread_assignment_e2e` + the store suite (both backends) green.
- **Limit (tracked):** the assignment mutations (assign/unassign/claim/claim_next
  → `ThreadAssignmentChanged`), DM/group-DM posts, and the slash-edit-entangled
  message post still use the retry-hardened `publish()`. Assignments are next
  (Cluster 209, reusing `thread_scope_in_tx`).

## References

- [[Retros/Cluster 208.0]]; `store/*/{thread_transitions,events}.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 207.0]].
