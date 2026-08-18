# Cluster 235.0 — structured-results REST + `ThreadResultSet` event

> Program B (agentic orchestration), part 19. **Arc F — coordination waits +
> structured results**. Phase XXIV post-gate hardening. Tag **`v235.0.0`**. No new
> gate tag.

## Goal

Wire the Cluster-234 thread-result store over **REST**, and make a result *set*
**observable**. The DAG's `ThreadReady` (222) tells a parent *when* a child finishes;
this makes the *what* — the child's structured output — writable, readable, and
event-driven. A `ThreadResultSet` event on set lets a waiter subscribe instead of
poll, exactly like `ThreadReady`.

## Scope

| Change | Where |
|--------|-------|
| `PUT /threads/:id/result` (`thread:transition`, upsert) + `GET /threads/:id/result` (`workspace:read`, `404` until produced) | `routes/thread.rs`, `dto.rs`, `app.rs` |
| `ThreadResultSet` event on set — the 11-site EventKind drill | `maidan-types/src/events.rs`, `federation.rs`, `event_kinds_contract.rs`, `contracts/event-kinds.json` |
| New-route preflight (OpenAPI stubs + `paths`/`components` regs, `http-capability-map.json` × 2, matrix PUT body clause) | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **`thread:transition` to set, `workspace:read` to get.** Producing a task's result
  is a task-lifecycle write (same cap as the FSM transition + assignment surfaces);
  reading it is a plain workspace read. Both run the DM-participant-aware
  `ensure_thread_access` (Cluster 180), so a private-channel / DM result stays scoped
  to members.
- **The event is a "go fetch" pointer, not the payload.** `ThreadResultSet` carries
  `{workspace, channel, thread, produced_by}` — no result inline. A result can be an
  arbitrarily large JSON blob; broadcasting it to every subscriber would bloat the
  event stream. The waiter learns *that* a result landed and fetches it with
  `GET …/result`. This mirrors the Cluster-178 lean-frame philosophy and `ThreadReady`.
- **Non-federatable.** A result is produced locally; a peer must not inject one. The
  `federatable()` allowlist (Cluster 215) excludes it alongside `ArtifactUpserted` +
  `ThreadReady` — enforced by the exhaustive-match tripwire.
- **Best-effort `super::publish`, not `*_with_event`.** `ThreadResultSet` is a derived
  notification (a signal that the store already committed), like `ThreadReady` — not
  a transactional-outbox event tied to the write. The store `set_thread_result` (234)
  is the durable record; the event is at-most-once realtime.
- **`GET` → `404` until produced.** No result row → `NotFound`, so a poller/waiter
  gets an unambiguous "not yet" (not an empty 200).

## Non-goals / deferred (Cluster 236, closes Program B)

- **MCP** `set_thread_result` / `get_thread_result` tools.
- **`wait_for_result`** long-poll (the coordination wait, the `wait_for_ready` shape) —
  block until a thread's result lands (subscribe to `ThreadResultSet`).
- A **"read my dependencies' results"** aggregate for a parent task (walk the DAG deps,
  gather each result).

## Risks

- Adding an EventKind is the 11-site drill (memory: the `ThreadReady` / 171 lesson) —
  a miss reds the `event_kinds_contract` test or the store read-back. Mitigated by
  running `maidan-types` + the federation remap test + the contract test.
