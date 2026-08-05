# Cluster 155.0 retro — sampling-backed `summarize_thread`

> Tag **`v155.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Lane 3 part 2 — closes the three-lane next-arc plan.

## What shipped

- **`summarize_thread`** — the first organic caller of `request_client`. A
  `tools/call` gathers the thread transcript and issues a server→client
  `sampling/createMessage` over the canonical GET stream (Cluster 154 delivery),
  returning the client's completion. `workspace:read`; `limit` clamped `1..=500`;
  optional `instructions`.
- **Session id threaded through tool dispatch.** `handle` → `handle_in_session`
  → `dispatch` → `tools_call` → `tools::dispatch` now carry an optional
  `Mcp-Session-Id`; the `POST /mcp/streamable` JSON-accept path and both SSE
  session paths pass it; non-streamable transports pass `None`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Agentic-features arc | HITL approvals (`request_approval` over elicitation) | A distinct feature; scheduled in the post-v155 program. |
| n/a | Streaming partial summaries | `sampling/createMessage` here is single-shot; fine for a summary. |

## Surprises

- **Dispatch was session-blind by design.** Every prior tool was
  transport-agnostic, so `handle` never carried a session id — a deliberate
  simplicity that had to be undone (minimally, via an optional param) for the
  first tool that talks *back* to its client. The JSON-accept POST path turned
  out to be the cleanest client ergonomics: the sampling request rides the GET
  stream while the tool result returns in the POST body.

## Decisions

- **`handle_in_session` + delegate `handle`→`None`** rather than changing
  `handle`'s signature everywhere — keeps every non-streamable caller untouched.
- **Client-side sampling, not server-side LLM.** `summarize_thread` asks the
  client to run the completion (MCP sampling), so the server needs no model
  credentials — the right trust boundary for a multi-tenant backend.

## Capability table extension

| Capability | Where |
|------------|-------|
| `summarize_thread` (sampling-backed) | `crates/maidan-mcp/src/tools/thread.rs` |
| Session-aware tool dispatch | `server.rs`, `mcp_streamable.rs` |

## Risks identified + still open

- **Low.** Additive tool + an optional dispatch param. The tool errors cleanly
  without a sampling-capable session; no existing tool behavior changes.

## Forward look — the three-lane plan is done; a new program begins

Token efficiency (151+152), live UI (153), and `request_client` (154+155) are
all shipped. A 5-agent research sweep (feature-gaps, performance, CI/CD, token,
production-readiness) then set the **post-v155 program — four arcs, in order**:
**(1) Enterprise hardening** (quick-wins → the flagship **channel/thread RBAC**,
the #1 finding: authz is workspace-flat today), **(2) Perf + CI/CD** (localized
DB batch-fixes + native-arm64 / build-once CI speedups), **(3) Agentic features**
(structured content, MCP backpressure, HITL approvals, task handoff), **(4)
Token round 3**. Detail in [[Roadmap]] and memory `maidan-next-arc-program`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
