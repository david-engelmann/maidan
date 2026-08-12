# Cluster 197.0 — agentic: tool-call transcripts (Arc C finale)

**Theme:** Arc C (agentic task-queue depth), part 8 — correlate the Cluster 173
`ToolUse`/`ToolResult` content blocks scattered across a thread's messages into a
single readable, token-lean transcript.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v197.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ToolTranscript`/`ToolCallEntry`/`ToolCallResult`/`OrphanToolResult` types + `tool_transcript(thread_id, &[Message])` extractor | `maidan-types/src/models.rs` |
| REST `GET /threads/:id/tool-transcript` (+ `ToolTranscriptQuery`) | `routes/thread.rs`, `dto.rs`, `app.rs` |
| MCP `get_tool_transcript` tool | `tools/thread.rs` + `mod.rs` + `catalog.rs` |
| OpenAPI path + schema regs; contracts (`http-capability-map`, both `mcp-*`) | `openapi/*`, `contracts/*` |

## Why

Cluster 173 gave messages a typed `content` axis with `ToolUse { id, name, input }`
and `ToolResult { tool_use_id, content, is_error }` blocks. But nothing *read* them
back as a unit: a multi-step agent's tool calls and their results were scattered
across message bodies, uncorrelated. An agent resuming work — or auditing what
another agent did — had to re-read every message and pair blocks by hand. The
transcript is the missing projection: "what tools were called in this thread, with
what inputs, and what came back."

## The design

`tool_transcript` is a pure two-pass extractor over a thread's (chronological)
messages:

1. **Pass 1** collects every `ToolUse` into an ordered `entries` list, indexed by
   id (a duplicate id keeps the first — later uses aren't distinguishable for
   correlation).
2. **Pass 2** attaches each `ToolResult` to its matching entry by `tool_use_id`.
   A result with no matching call — or a second result for an already-resolved
   call — becomes an `orphan_result` (surfaced, not dropped, so a gap is visible).

Correlation is **order-independent** (a result may land in a later message than
its call), and tombstoned messages are skipped (their content is gone). The result
is a **token-lean projection**: `Text`/`Code`/`ResourceLink` blocks and the
message `body` are dropped — only the tool structure remains. Both surfaces clamp
`limit` to 1..=500 (default 200) and enforce thread-RBAC (`ensure_thread_access`
on REST, the pre-dispatch `thread_id` gate on MCP).

## Exit criteria

- A thread's `ToolUse`/`ToolResult` blocks are correlated into a transcript on
  REST + MCP; a non-member of a private channel is denied — **met**.
- **Arc C (agentic task-queue depth) is complete** (190–197).
- `v197.0.0` tagged.

## Verification & limits

- `maidan-types` unit tests: pairs-across-messages, unresolved-call +
  orphan-result, tombstoned-skip.
- `tool_transcript_e2e`: REST + MCP return the correlated transcript for the same
  thread; a non-member is `403` on the private-thread transcript.
- New-route preflight: OpenAPI path stub + `paths(...)` + `components(schemas())`
  (4 new types) + `http-capability-map` GET entry → `openapi_e2e` bijection green
  (GET, so no `http_capability_matrix` body clause needed); both MCP contract-sync
  tests green.
- Limit: bounded by `limit` (≤ 500 messages scanned) — a transcript of a thread
  with more tool-bearing messages than the window is truncated (documented; a
  cursor/paginated transcript is a possible follow-up). A result whose call is
  outside the window shows as an orphan.

## References

- [[Retros/Cluster 197.0]]; `maidan-types/src/models.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc C, final). Builds on
  [[Retros/Cluster 173.0]] (structured content).
