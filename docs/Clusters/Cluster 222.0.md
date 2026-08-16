# Cluster 222.0 — reactive task readiness (`ThreadReady`)

> Program B (agentic orchestration), part 6. Phase XXIV post-gate hardening.
> Tag **`v222.0.0`**. No new gate tag.

## Goal

Make DAG readiness *reactive*. Clusters 217–221 made readiness a pull: an agent
had to call `dependencies_satisfied` / `list_thread_dependencies` (or lean on
`claim_next`'s readiness filter) to discover a task became workable. This cluster
pushes it: when a thread's last blocking dependency reaches a terminal state, the
newly-unblocked dependents emit a `ThreadReady` event, so a waiting agent can
subscribe instead of poll.

## Scope

| Change | Where |
|--------|-------|
| `EventKind::ThreadReady` + `Event::ThreadReady { workspace_id, channel_id, thread_id, thread }` — plus all accessor/round-trip arms; **non-federatable** (locally derived) | `maidan-types/src/events.rs` |
| `Store::newly_ready_dependents(thread_id)` — non-terminal dependents now fully unblocked (recursive `NOT EXISTS` over deps), both backends | `store.rs`, `store/{sqlite,postgres}/thread_deps.rs`, `store/{sqlite,postgres}/mod.rs` |
| Transition route emits `ThreadReady` (via `publish`) for each newly-ready dependent, guarded on a non-terminal → terminal edge | `routes/thread.rs` |
| Federation remap arm (exhaustive match) + contract `event-kinds.json` + contract test | `federation.rs`, `contracts/event-kinds.json`, `event_kinds_contract.rs` |

## Design decisions

- **Emit in the route via `publish()`, not in `transition_thread_with_event`.**
  `ThreadReady` is a *derived standalone* event — there is no domain-table row it
  must be atomic with (readiness is computed from already-committed thread states).
  That's exactly the `publish()` contract (like `publish_routed_mentions`), and it
  avoids changing the transition store method's return type (`(result, event)` →
  N extra events would ripple to every caller).
- **Guard on the transition edge, not the resulting state.** Only a
  non-terminal → terminal transition can unblock a dependent, so `closed→archived`
  (terminal→terminal) does not re-emit. Uses `ThreadTransitionResult`'s
  `from_state`/`to_state`.
- **Non-federatable.** Readiness is a function of *this* deployment's dependency
  graph + thread states; a peer must not inject it. Classified `false` in
  `EventKind::federatable()` (the second exception after `ArtifactUpserted`).
- **No new consumer tool this cluster.** The event rides the existing WS / MCP-SSE
  subscribe (filter `kinds=thread_ready`, RBAC-filtered by the standard fan-out). A
  dedicated `wait_for_ready` MCP long-poll (the `wait_for_mention` analogue) is a
  natural follow-up.

## Non-goals / deferred

- A blocking `wait_for_ready` MCP tool (subscribe covers it for now).
- Emission from any future non-route transition path (there is only one FSM
  mutation call site today; a shared helper can be lifted if that changes).

## Risks

- **Best-effort emit.** A store/bus hiccup during emission is logged, not
  surfaced — the transition already committed and readiness stays queryable, so the
  agent's `claim_next` still works. At-least-once via the durable event log if the
  subscriber uses `at_least_once`.
