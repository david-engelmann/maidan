# Cluster 193.0 — agentic: the `roots/list` tool

**Theme:** Arc C (agentic task-queue depth), part 4 — give `request_client`'s
third verb its first organic caller: an MCP tool that asks the client which
**roots** it exposes.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v193.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `list_roots` MCP tool (server→client `roots/list`) | `maidan-mcp/src/tools/roots.rs`, `mod.rs`, `catalog.rs`, contracts |

## Why

Cluster 148 wired `request_client` (server→client requests over the streamable
session) and gated it on the client's declared capability. Two verbs got organic
callers — sampling (`summarize_thread`, 155) and elicitation (`request_approval`,
174) — but the third, **`roots/list`**, had none: the transport existed, nothing
used it. `roots/list` is the discovery primitive by which the server asks the
client for its filesystem/workspace boundaries.

## The fix

`list_roots` issues `request_client(session, "roots/list", {})` and returns the
client's `{roots: [...]}` verbatim — the exact shape of `request_approval`, minus
any args. It requires a streamable session whose client declared the `roots`
capability (enforced by `request_client`); without a session it returns a clear
`InvalidParams`. Capability `workspace:read` (a discovery read).

## Exit criteria

- An agent on a `roots`-capable streamable session can query the client's roots;
  the request rides the GET stream and the response resolves the tool call —
  **met**.
- `v193.0.0` tagged.

## Verification & limits

- `mcp_streamable_e2e::list_roots_tool_queries_the_client`: open a session
  declaring `roots` → GET stream → call `list_roots` → the `roots/list` request
  arrives on the stream → the client answers `{roots:[{uri,name}]}` → the tool
  resolves with those roots. Contract-sync (`mcp-tool-names` /
  `mcp-capability-map`) green.
- Limit: like the other `request_client` tools, this needs the **streamable**
  transport (`GET /mcp/streamable`) with a `roots`-declaring client — it does
  nothing over the plain `POST /mcp`. Maidan doesn't yet *use* the returned roots
  for anything server-side; it's a discovery primitive exposed to agents.

## References

- [[Retros/Cluster 193.0]]; `maidan-mcp/src/tools/roots.rs`. Program: [[Roadmap]]
  + memory `maidan-next-arc-program` (Arc C). Transport: [[Retros/Cluster 148.0]];
  sibling callers [[Retros/Cluster 155.0]] + [[Retros/Cluster 174.0]].
