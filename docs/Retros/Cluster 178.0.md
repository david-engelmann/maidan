# Cluster 178.0 retro — opt-in lean event frames

> Tag **`v178.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 4 (token round 3), part 4 — **the final item. Token round 3 and the
> post-v155 four-arc program are COMPLETE.**

## What shipped

- A `lean` subscribe flag (WS `SubscribeFrame` + MCP-SSE query) threaded through
  `forward_bus_items` / `replay_matching_events` / `reconcile_deliver`. When set,
  a new `frame_payload(envelope, lean)` emits a `LeanFrame`
  (`{log_id, kind, workspace_id?, channel_id?, thread_id?, member_id?}`) instead
  of the full flattened event — a "something happened, go fetch" pointer.

## Surprises

- **`at_least_once` was the template.** The flag threads through the exact same
  three delivery functions + two transports that `at_least_once` already does,
  so the plumbing was mechanical. Centralizing the choice in one
  `frame_payload` helper meant the three serialize sites each changed by one call.
- **The flatten made it drop-in.** Because a full frame is the flattened
  `{log_id, kind, …}`, the lean frame is a *strict subset* of its top-level
  fields — the Cluster 153 `/ui` live-refresh (`frame.thread_id` + `frame.kind`)
  and any `typeof log_id === "number"` check work identically on a lean frame.

## Decisions

- **Fixed pointer shape, opt-in, all paths.** No per-kind projection; default
  off; optimistic + replay + reconcile all route through `frame_payload`.

## Capability table extension

| Change | Where |
|--------|-------|
| `lean` subscribe flag → `LeanFrame` event pointers (WS + MCP SSE) | `event_stream.rs`, `ws.rs`, `mcp_stream.rs` |

## Risks identified + still open

- **Low.** Opt-in; default frames unchanged (verified — all frame-consuming
  suites green). A lean tail requires the client to fetch for bodies, which is
  the intended trade.

## Forward look — the four-arc program is done

Post-v155, the approved program was: **(1) enterprise hardening** (156–165:
prod-safety defaults, fail-closed auth, signed images, the channel/thread RBAC
arc), **(2) perf + CI/CD** (166–170: R1/R2/R3 + H1/H2/H4/H6, native arm64
release + trivy), **(3) agentic features** (171–174: task assignment, MCP
backpressure, structured content, HITL approvals), **(4) token round 3**
(175–178: MCP snippet_only, capability-filtered tools/list, omit-empty metadata,
lean event frames). **All four arcs are complete.** The next program is
unset — a fresh research sweep across [[Open Work]] / [[Remaining Work]] should
choose it (candidate threads: federation `parts↔content` propagation deferred in
173, persisted/auditable HITL approvals deferred in 174, RLS defense-in-depth
deferred in the RBAC arc, and a docs link-checker).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
