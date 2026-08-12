# Cluster 197.0 retro — a thread's tool calls, correlated (Arc C done)

> Tag **`v197.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 8 — the finale.

## What shipped

- `tool_transcript`: a pure extractor that pairs a thread's `ToolUse` blocks with
  their `ToolResult` blocks by id and returns a token-lean `ToolTranscript`.
  Exposed as REST `GET /threads/:id/tool-transcript` and MCP `get_tool_transcript`
  (both `workspace:read`, thread-RBAC). **Completes Arc C** (190–197).

## Surprises / decisions

- **The data existed since 173; only the projection was missing.** This cluster
  added no new column, event, or capability — the `ToolUse`/`ToolResult` blocks
  already ride on `Message.content`. The value is *correlation done once,
  server-side, token-lean*: an agent gets the tool structure without re-reading
  every body and pairing blocks by hand. That framing kept the cluster tight (a
  pure function + two thin surfaces).
- **Order-independent correlation, not "walk and match".** A first instinct is to
  match a result to the most-recent unmatched use as you walk. But a `ToolResult`
  can land in a later message than its `ToolUse`, and ids are the real key — so
  two passes (collect uses by id, then attach results) is both simpler and
  correct regardless of message order. It also gives clean orphan detection: a
  result whose id isn't in the use-index is surfaced, not silently dropped.
- **Orphans are a feature, not an error.** A `ToolResult` with no matching call in
  the scanned window (or a duplicate result) goes to `orphan_results` rather than
  failing. A truncated window or a cross-thread reference is a real, visible gap —
  swallowing it would hide it.
- **Tombstoned messages are skipped.** A redacted message's content is gone, so
  its tool blocks shouldn't appear (and a tombstoned *call* leaves its result an
  orphan — the test asserts exactly that).

## Decisions

- **Pure extractor in maidan-types**, so REST and MCP share one implementation and
  it's unit-testable without a store or HTTP.
- **`workspace:read` + thread-RBAC on both surfaces.** A transcript is a read of
  thread content; it inherits the same access rules as `list_messages`/context —
  a non-member of a private channel is denied (e2e asserts the `403`).
- **`limit` clamped 1..=500 (default 200)**, mirroring the context tools — a
  transcript is a projection of a bounded message window, not an unbounded scan.

## Capability table extension

| Change | Where |
|--------|-------|
| Tool-call transcript: `tool_transcript` + REST `GET /threads/:id/tool-transcript` + MCP `get_tool_transcript` | `maidan-types` + `routes/thread.rs` + `tools/thread.rs` |

## Risks identified + still open

- **Net additive, non-breaking** — a new read surface over existing data; no write
  path touched. Open: the transcript is bounded by `limit` (a very long thread is
  truncated — a paginated/cursored transcript is the follow-up), and a call whose
  result is outside the window shows as an orphan.

## Forward look

**Arc C (agentic task-queue depth) is complete** (190 assignment read-side, 191
MCP tools, 192 claim leases, 193 `list_roots`, 194 A2A `parts→content`, 195
handoff notes, 196 `wait_for_mention`, 197 tool-call transcripts). Next is **Arc D
— performance & scale**: a load/soak harness first (measure before optimizing),
then workspace-sharded fan-out + shared reconcile, filtered-ANN search, batched
`pg_notify`, read-replica routing, and batched context assembly. Deferred from
Arc C: federation `content→parts` *egress* (194 shipped ingest; egress is still
body-only).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on
[[Retros/Cluster 173.0]] (structured message content).
