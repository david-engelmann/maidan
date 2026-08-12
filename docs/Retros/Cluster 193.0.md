# Cluster 193.0 retro — the third request_client verb finally has a caller

> Tag **`v193.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 4.

## What shipped

- `list_roots` — an MCP tool that issues the server→client `roots/list` request
  and returns the client's roots. Handler + dispatch + capability + catalog + both
  contracts + a full-round-trip e2e.

## Surprises / decisions

- **Genuinely near-free, as advertised.** All the hard parts — the
  `request_client` transport, GET-stream delivery, capability gating, session
  threading through `dispatch` — landed in 148/154/155. This cluster is a ~30-line
  handler plus the standard five-place tool wiring. The value is that the third
  verb is no longer dead transport code.
- **The e2e is the real proof, and it mirrors exactly.** The
  `request_approval`/`summarize_thread` streamable tests already establish the
  pattern (open session with the capability → GET stream → call tool → intercept
  the server→client request on the stream → answer it → assert the tool
  resolves). `list_roots` slots straight into it — same six steps, different verb
  and payload.

## Decisions

- **Return the client's `{roots}` verbatim.** No server-side interpretation —
  Maidan doesn't consume the roots yet; the tool is a discovery primitive an agent
  can call. Wrapping or reshaping would presume a use that doesn't exist.
- **`workspace:read`, session-gated.** Discovery is a read; the real gate is the
  client having declared `roots` (enforced by `request_client`), so a session-less
  call fails fast with a clear message.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `list_roots` (server→client `roots/list`) | `maidan-mcp/src/tools/roots.rs` |

## Risks identified + still open

- **Net additive, zero blast radius** — a new opt-in tool on the streamable
  transport; the plain `POST /mcp` path is untouched. Open: nothing server-side
  consumes the roots yet (it's exposed for agents to use).

## Forward look

Arc C continues: structured tool-call transcripts, `wait_for_mention` blocking
primitive, handoff notes, federation `parts→content`. Then Arc D (performance &
scale).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Transport:
[[Retros/Cluster 148.0]]; siblings [[Retros/Cluster 155.0]] + [[Retros/Cluster 174.0]].
