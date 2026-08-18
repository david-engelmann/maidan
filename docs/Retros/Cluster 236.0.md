# Cluster 236.0 retro — agents wait on and aggregate task results (Program B closes)

> Tag **`v236.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 20 — **closes Arc F and Program B**.

## What shipped

- MCP `set_thread_result` / `get_thread_result` — the twins of 235's REST, over the
  shared Cluster-234 store.
- MCP `wait_for_result` — a long-poll that blocks on a thread's `ThreadResultSet`
  event and returns the result *payload*.
- MCP `get_dependency_results` — a parent task reads its dependencies' outputs as
  `[{thread_id, result}]`.

With these, the "spawn sub-tasks, wait, aggregate" loop is expressible entirely over
MCP: build the DAG (220), claim ready work (218/191), set a result on finish
(236), `wait_for_result` / `wait_for_ready` on the coordination edge (223/236), and
`get_dependency_results` to gather. **Program B is complete.**

## Surprises / decisions

- **The wait returns the payload; the ready-wait returns the event.** It's a small
  asymmetry with `wait_for_ready` but the right one: readiness is itself the signal
  (there's nothing else to fetch), whereas a result *is* a payload, so making the
  caller do a second `get_thread_result` after the wait would be busywork. The
  thread-pinned event filter means the arriving frame is unambiguously the awaited
  one, and the pre-dispatch gate already proved access — so the handler is just
  "subscribe, wait, fetch, return," no per-event RBAC loop.
- **Project the payload, don't echo the envelope.** The first cut of
  `get_dependency_results` returned the whole `ThreadResult` under `result`, which
  double-nested `thread_id` — the test caught it immediately (`left`/`right`
  mismatch). Mapping to `r.result` gives the clean `{thread_id, result}` shape a
  parent actually wants; `null` marks a not-yet-produced dependency, which is more
  useful than omitting it (the parent sees *which* children are still pending).
- **Four small tools, one gate arm.** All four key on `thread_id`, so they slotted
  into the existing `ensure_thread_access` pre-dispatch arm with no new gate logic —
  the aggregate's cross-channel dependencies get the same in-handler
  `can_access_thread` filter `list_assigned_threads` established. The 5-place drill
  (handler + dispatch + capability + gate + catalog) plus both sorted contracts went
  in clean; the contract-sync tests are the safety net for the sorted-JSON edits.
- **A dependency fix rode along in 235, not here.** A freshly-published advisory
  (RUSTSEC-2026-0258, h2 empty-DATA-frame DoS) red the `cargo deny` gate mid-235; the
  first-party hyper-1.x surface was patched (h2 0.4.16) and the residual AWS-SDK
  h2 0.3.27 triaged in `deny.toml` with the existing aws-sdk precedent. Noting it
  here because it's the freshest "the toolchain moves under you" reminder — a green
  local run isn't a guarantee once the advisory DB updates.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `set_thread_result` / `get_thread_result` / `wait_for_result` / `get_dependency_results` | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None new. The result surface is now complete over REST (235) + MCP (236).

## Forward look

**Program B (agentic orchestration) is complete** — the task-DAG + queue subsystem
(217–225), scheduled/recurring tasks (226–229), the capability registry + skill
routing (Arc E, 230–233), and coordination waits + structured results (Arc F,
234–236). Next: **Program C (notifications & reach)** — a per-recipient router +
inbox, routing prefs + presence-aware delivery, an email/SMTP transport, digests,
and follow/UI — then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 235.0]].
