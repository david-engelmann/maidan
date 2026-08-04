# Cluster 150.0 — MCP stream thread/member/kind filters

**Theme:** Second of the **MCP-agent-surface arc**. Let an MCP/SSE agent
narrow `GET /mcp/stream` server-side to a channel / thread / member / kind —
the "await my mention" primitive — instead of filtering the whole workspace
client-side.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v150.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Add `channel_id`, `thread_id`, `member_id`, `kinds` query params | `mcp_stream.rs::McpStreamQuery` |
| Wire them into the `EventFilter` (kinds via `EventKind::parse`; unknown → 400) | `mcp_stream.rs::resolve_stream_params` |

## Why

`GET /mcp/stream` only wired `workspace_id` / `dm_conversation_id` /
`channel_grants`, while the WebSocket subscribe accepted the full
`EventFilter`. `EventFilter::matches` already filters on
channel/thread/member/kinds (member_id covers `MentionRecorded`). So an
MCP/SSE agent couldn't subscribe to "just my mentions" or one thread — it had
to take the whole workspace firehose and filter client-side. This is pure
query→filter wiring; the payoff is
`?workspace_id=…&member_id=…&kinds=mention_recorded`.

## Non-goals

- New filter *fields* — all already exist on `EventFilter`; this only exposes
  them on the MCP/SSE query surface.
- WebSocket changes — the WS already had these via its JSON `filter`.

## PR ladder (actual)

| # | Title |
|---|--------|
| 150.0.1 | `feat(mcp): thread/member/kind filters on GET /mcp/stream` (#390) |
| 150.0.retro | `docs(retro): Cluster 150.0 + v150.0.0 tag prep` |

## Exit criteria

- The four params narrow live delivery; unknown kind → 400; tests green —
  **met**.
- `v150.0.0` tagged after retro.

## Verification & limits

- E2E: `mcp_stream_filters_by_event_kind`, `mcp_stream_rejects_unknown_kind`.
  `fmt`/`clippy` clean; cap-map unchanged (same route, new params); OpenAPI
  description updated.
- Limit: `kinds` is a comma-separated string (axum `Query` is
  `serde_urlencoded`, no repeated-key arrays) — a deliberate, simple encoding.

## References

- [[Retros/Cluster 150.0]]; [[Clusters/Cluster 149.0]]; `mcp_stream.rs`,
  `crates/maidan-types/src/events.rs` (`EventFilter::matches`, `EventKind::parse`).
