# Cluster 369 retro — Wave 2 #17: an AG-UI door (H1)

Wave 2 #17 opens a second front-end protocol on Maidan's event stream:
[AG-UI](https://docs.ag-ui.com), the Agent-User Interaction protocol that
CopilotKit and a growing set of agent IDEs speak. The bet is small and cheap: a
**thread is a run**, and Maidan already emits every state change on a resumable
bus — so the "door" is a *view* over the existing events, not a new runtime.

## What shipped

- **369.1 (#731) — the event types + pure mapping.** `AgUiEvent` (the AG-UI wire
  shape: a `SCREAMING_SNAKE_CASE` `type` tag + `camelCase` fields) and a pure
  `agui_events_for(&Event) -> Vec<AgUiEvent>` in `crates/maidan-server/src/agui.rs`.
  `ThreadCreated` → `RUN_STARTED` (runId = threadId), a terminal
  `ThreadStateChanged` → `RUN_FINISHED` and a non-terminal one → `STEP_STARTED`,
  `ClaimFailed` → `RUN_ERROR`, `MessagePosted` → `TEXT_MESSAGE_START/CONTENT/END`
  plus a `TOOL_CALL_*` triple per `ToolUse` block and a `TOOL_CALL_RESULT` per
  `ToolResult`, `ThreadLanded` → a `CUSTOM` event; everything else maps to
  nothing. Five unit tests pin it.
- **369.2 (#733) — the SSE endpoint.** `GET /agui/stream` subscribes to the event
  bus (workspace / channel / thread scoped) and emits the mapped frames. It
  reuses the `/mcp/stream` machinery — bus subscribe + `envelope_from_stored`
  replay — rather than a parallel stack. Resume is SSE-standard (`Last-Event-ID`
  header or `after_id` query → replay past what was seen; every frame carries its
  source event-log `id:`). Each event is RBAC-filtered per recipient
  (`can_access_thread` / `can_access_channel`), and a bus lag surfaces as a
  `CUSTOM` `lagged` frame.

## Decisions

- **A thread is a run.** AG-UI's core object is a "run"; Maidan's is a thread.
  Mapping runId = threadId means a UI's run lifecycle is Maidan's thread
  lifecycle with no bookkeeping — and the mapping stays a pure function of one
  event, so it's fully unit-tested with no store.
- **A view, not a stack.** The door reuses the resumable bus + replay behind
  `/mcp/stream` verbatim (`envelope_from_stored`, `REPLAY_LIMIT`). Building a
  parallel subscribe/resume path would have doubled the surface for zero gain —
  the same events, framed differently.
- **Off-contract like `/mcp/stream` and `/scim/v2`.** A streaming SSE route
  isn't a request/response OpenAPI operation, so `/agui/stream` lives in the
  `protected` router with auth (`event:subscribe`) enforced inline — in neither
  OpenAPI nor the capability map, so the bijection stays intact with no stub.
- **Per-event RBAC, not subscribe-grants.** The forwarder filters each event by
  `can_access_thread` / `can_access_channel` (bypass exempt), so a workspace-wide
  AG-UI stream never leaks a private channel — reusing the tested access helpers
  rather than the asserted-grants plumbing an AG-UI client wouldn't send.
- **Output direction only.** Wave 2 #17 scoped the input direction (a UI sending
  interrupts, `editedArgs` = full replace) as a follow-up; REST already carries
  those mutations. F3 (agent↔tool) stays MCP MRTR — this is the human-UI door.

## Surprises

- **`after_id=0` means live-only, not "replay everything."** The `/mcp/stream`
  convention triggers replay on `after_id > 0`; 0 is a fresh live subscribe. My
  first resume test wrongly expected `after_id=0` to replay the backlog and
  timed out — rewritten to capture a live frame's id and reconnect with
  `Last-Event-ID`, which is the real resume path anyway.
- **`cargo fmt` re-sorts the `mod` list.** `pub mod agui;` inserted next to
  `consistency` jumped to its alphabetical slot after `a2a_grpc` on the first
  fmt — expected (memory: fmt reorders), just noted so the diff reads right.

## Test evidence

- `maidan-server` lib: `agui` unit tests (5) — run start/finish/step, run error,
  the text + tool-call decomposition, the empty-body omission.
- Server e2e: `agui_stream_e2e` (3, SQLite) — a thread lifecycle → `RUN_STARTED`
  then `TEXT_MESSAGE_START/CONTENT/END` with per-frame ids; a `Last-Event-ID`
  reconnect replays only the unseen run; `after_id` without a workspace → `400`.
- RBAC composes the separately-tested `can_access_*` helpers (bypass in these
  e2es; the denial paths are `channel_access_e2e` / `subscribe_grants_e2e`).

## Forward look

**Wave 2 #17 is complete.** Deferred (follow-ups): the **input direction** — a UI
POSTing AG-UI `RunAgentInput` / interrupts / `editedArgs` = full replace (REST
covers the mutations today); richer `STEP_*` semantics (a step per FSM edge is
coarse); an AG-UI-native `/ui` panel (the door is protocol-only). **ACP** (#51,
the footnote on H1) stays skipped — AG-UI is the HITL door. **Next: Wave 2 #18.**

## Acknowledgements

Two impl PRs (#731 types+mapping → #733 SSE, reopened after the stacked base
merged) plus this retro, on the off-contract-streaming-route (`/mcp/stream`) and
per-event-RBAC (`wait_for_ready`) patterns.
