# Cluster 148.0 — MCP server→client requests (arc 145–148 conclusion)

**Theme:** The final slice of the **MCP streamable spec-completeness arc
(145–148)** — the bidirectional gap. The server can issue JSON-RPC *requests*
to a client (sampling / roots / elicitation) and await the response.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v148.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Per-session client-capability tracking (from `initialize`) | `mcp_streamable.rs` (capture), `streamable_session.rs` (`set_client_capabilities` / `client_supports`) |
| `McpServer::request_client(session, method, params)` — outbound request + pending-reply correlation, capability-gated | `server.rs`, `streamable_session.rs` (`register_client_request` / `resolve_client_response`) |
| Inbound response routing — `POST /mcp/streamable` distinguishes a JSON-RPC response from a request | `mcp_streamable.rs::streamable` |

## Design

- The correlation map (`pending: HashMap<id, oneshot::Sender>`) lives in the
  session entry, so `request_client` is: check capability → allocate id +
  oneshot → push the request onto the session SSE → await (30s timeout). The
  client's response arrives as an inbound POST; the handler detects it
  (`id` present, `method` absent) and resolves the oneshot.
- Client capabilities are captured in the handler on `initialize` (keeps
  `McpServer::handle` transport-agnostic).

## Honest scope note

Maidan's server has **no organic caller** for `request_client` yet — it's
transport capability for future features (e.g. an agent tool that samples the
client's model). The arc's stated goal was **literal spec-completeness**; the
machinery, capability gating, and correlation are implemented and tested end to
end, ready for a caller.

## The arc, complete (145–148)

| Cluster | Gap closed |
|---------|-----------|
| 145 | `initialize` version negotiation; `MCP-Protocol-Version` header; JSON-RPC batching + notifications |
| 146 | `GET /mcp/streamable` server→client SSE + `Accept` content negotiation |
| 147 | Resumability — SSE `id:` + `Last-Event-ID` replay |
| **148** | **Server→client requests + client-capability tracking** |

## PR ladder (actual)

| # | Title |
|---|--------|
| 148.0.1 | `feat(mcp): server→client requests + client-capability tracking` (#386) |
| 148.0.retro | `docs(retro): Cluster 148.0 + v148.0.0 tag prep` |

## Exit criteria

- Server→client requests round-trip; capability gating enforced; inbound
  responses routed; tests green — **met**.
- `v148.0.0` tagged after retro; the MCP streamable spec-completeness backlog
  item is closed.

## Verification & limits

- Unit (maidan-mcp): registry correlation + capability; `request_client`
  happy-path + capability-denied. E2E:
  `server_to_client_request_round_trips_over_http` (full HTTP loop). `fmt` /
  `clippy` clean; OpenAPI + contract updated.

## References

- [[Retros/Cluster 148.0]]; [[Clusters/Cluster 145.0]]–[[Clusters/Cluster 147.0]];
  `maidan-mcp/src/server.rs`, `streamable_session.rs`, `mcp_streamable.rs`.
