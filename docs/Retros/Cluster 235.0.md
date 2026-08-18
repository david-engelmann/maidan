# Cluster 235.0 retro — a task's answer becomes observable

> Tag **`v235.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 19 — Arc F.

## What shipped

- `PUT /threads/:id/result` (`thread:transition`, upsert) + `GET /threads/:id/result`
  (`workspace:read`, `404` until produced) — the REST wiring for the Cluster-234
  store — both under the DM-participant-aware thread RBAC.
- A `ThreadResultSet` **event** on set: a "go fetch" pointer (`{workspace, channel,
  thread, produced_by}`, no payload inline), observable on the WS + MCP-SSE event
  streams like `ThreadReady`. Non-federatable (locally derived).

## Surprises / decisions

- **The pointer-event shape is the whole design.** The tempting move is to put the
  result in the event so a subscriber gets it for free. But a result is
  arbitrary-size JSON, and the event stream fans out to every subscriber — a big
  result would bloat every frame. `ThreadResultSet` says *that* a result landed;
  `GET …/result` fetches it. This is the same lean-frame logic as Cluster 178, and it
  keeps the event cheap and the payload pull-based. It also composes cleanly with
  236's `wait_for_result` — the long-poll returns the pointer, the agent fetches.
- **`ThreadReady`'s twin, one arc later.** 222 shipped `ThreadReady` (a *when*); this
  is `ThreadResultSet` (a *what*). Both are derived signals off a store write, both
  use `super::publish` (best-effort, at-most-once) rather than the transactional
  outbox, and both are non-federatable. Having the `ThreadReady` template made the
  11-site EventKind drill mechanical — enum + `as_str` + `parse` + `ALL` +
  `federatable` + the `Event` variant + six accessors + the round-trip tripwire + the
  federation remap arm + the contract test + `event-kinds.json`. No site missed;
  the tripwire match and the `federatable` test both name the new variant, so a miss
  would have failed to compile or failed the test rather than rolling back an insert
  silently (the 171 failure mode, now designed out by 181's single-`parse` survivor).
- **`thread:transition` to write, `workspace:read` to read.** Setting a result is a
  task-lifecycle act — the same capability that transitions the FSM and assigns the
  thread. Reading is a plain workspace read. The split falls straight out of the
  existing capability vocabulary; no new cap, so no capability-matrix churn.
- **The e2e needs a real member.** `produced_by = auth.member_id` is a NOT-NULL FK, so
  the test mints a real token (auth enabled) rather than using bypass's nil member —
  the same pattern the skills / schedule / channel-access e2es established. The test
  also subscribes a bus filter on `ThreadResultSet` and asserts the set published it,
  proving the event end-to-end (not just the HTTP round-trip).

## Capability table extension

| Change | Where |
|--------|-------|
| `PUT`/`GET /threads/:id/result` + `ThreadResultSet` event (11-site drill) | `routes/thread.rs`, `dto.rs`, `app.rs`, `events.rs`, `federation.rs`, `openapi/*`, `contracts/*` |

## Risks identified + still open

- None new. The EventKind drill is the standing "add a variant" risk; it was covered
  by the `maidan-types` + federation-remap + contract tests, all green.

## Forward look

**236** closes Arc F — and **Program B** — with the MCP surface: `set_thread_result`
/ `get_thread_result` tools + a **`wait_for_result`** long-poll (block until a
thread's result lands, the `wait_for_ready` shape) + a "read my dependencies'
results" aggregate that walks a parent task's DAG deps and gathers each result. Then
Programs C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 234.0]].
