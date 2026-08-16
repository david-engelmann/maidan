# Cluster 222.0 retro — readiness stops being a poll

> Tag **`v222.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 6.

## What shipped

- `ThreadReady` — a new event emitted when a thread's last blocking dependency
  reaches a terminal state, unblocking dependents. An agent waiting on the DAG can
  now subscribe (`kinds=thread_ready`) instead of polling `dependencies_satisfied`.
  Backed by `Store::newly_ready_dependents` (both backends) and a guarded emit in
  the transition route.

## Surprises / decisions

- **A new event kind is a checklist, not a line.** `EventKind::ThreadReady` had to
  land in *eleven* places to compile + pass contracts: the enum, `as_str`, `parse`,
  `ALL`, `federatable`, the `Event` variant, its `kind`/`occurred_at`/`workspace_id`/
  `channel_id`/`thread_id` accessors, the federation `remap` arm, the round-trip
  tripwire match, the `federatable` test, and both the `event-kinds.json` contract
  file and its test's hardcoded list. The good news: every one of those is an
  *exhaustive match* or a *contract test*, so the compiler and CI walked me through
  the list — a missing arm is a build error, not a silent gap. (Cluster 181's
  collapse of the per-backend `parse_kind` copies means the store no longer adds
  three more places — the old Cluster-171 trap is gone.)
- **`publish()` was the right tool, and it's *still* not dead.** `ThreadReady` is a
  derived event with no domain row to be atomic with — the exact shape `publish()`
  serves (alongside the federation relay and mention routing). Emitting it in the
  route rather than folding it into `transition_thread_with_event` kept the store
  method's `(result, event)` signature intact; folding would have forced an N-event
  return type onto every transition caller.
- **Guard the *edge*, not the *state*.** Emitting whenever the thread is terminal
  would double-fire on `closed→archived`. `ThreadTransitionResult` already carries
  `from_state`/`to_state`, so `!from.is_terminal() && to.is_terminal()` is the exact
  unblock trigger — and it's free (no extra query).
- **Non-federatable, on principle.** Readiness is computed from local state; a peer
  asserting "this thread is ready" is meaningless (and a mild injection vector). It
  joins `ArtifactUpserted` as the second `federatable() == false` kind, and the
  allowlist test now checks a set rather than "everything but artifacts."

## Capability table extension

| Change | Where |
|--------|-------|
| `ThreadReady` event on dependency-unblock + `newly_ready_dependents` query | `events.rs`, `store/*/thread_deps.rs`, `routes/thread.rs` |

## Risks identified + still open

- **Best-effort emit** — a failed publish is logged, not surfaced; the transition
  is already committed and readiness stays queryable. `at_least_once` subscribers
  get the durable-log guarantee.
- **No `wait_for_ready` tool yet** — the event rides the generic subscribe; a
  blocking MCP long-poll (the `wait_for_mention` analogue) is the obvious follow-up.

## Forward look

Program B continues: scheduled/recurring tasks, a capability registry + skill
routing (match work to agents), queue-depth metrics, and coordination waits +
structured results. Then Programs C (notifications & reach) and D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 221.0]].
