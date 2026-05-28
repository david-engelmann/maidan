# Cluster 15.0 — MCP `resources/subscribe` notifications

Cluster 14.0 closed SQLite transactional outbox at **`v14.0.0`**. Maidan already exposes
`GET /mcp/stream` for workspace events (Cluster H), but the long-standing Cluster B deferral
is **MCP JSON-RPC `resources/subscribe`** — streaming resource updates over the MCP protocol
for desktop clients.

> **Epic pick:** **MCP `resources/subscribe`** (deferred since Cluster B retro).
>
> **Goal:** Implement `resources/subscribe` / `notifications/resources/updated` (stdio first)
> so MCP clients can watch Maidan resources without polling. Reuse existing mutation
> paths where possible; do not duplicate `/mcp/stream` event fan-out.
>
> **Target tag:** `v15.0.0`.

## Alternatives considered (not this cluster)

| Epic | Why deferred |
|------|----------------|
| SQLite semantic (`sqlite-vec`) | Extension maturity. |
| S3 multipart uploads | Cluster E follow-up; isolated from MCP. |

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 15.0 kickoff plan` (this doc)                           | —     |
| 15.0.1     | `feat(maidan-mcp): resources/subscribe handler + notification shape`   | TBD   |
| 15.0.2     | `feat(maidan-cli): stdio notification wiring + validation`             | TBD   |
| 15.0.3     | `test: MCP subscribe integration`                                      | TBD   |
| 15.0.4     | `docs: MCP subscribe in Architecture + MCP reference`                  | TBD   |
| 15.0.retro | `docs(retro): Cluster 15.0 + v15.0.0 tag prep`                          | TBD   |

## Order

1. **15.0.1** — JSON-RPC methods in `maidan-mcp`; resource URI scheme; notification payload.
2. **15.0.2** — stdio loop delivers notifications to subscribed clients.
3. **15.0.3** — integration tests (in-process dispatcher + at least one transport).
4. **15.0.4** — docs + generated MCP reference update.
5. **15.0.retro** + `v15.0.0` tag.

## Exit criteria

- CI green on `main`.
- MCP client can subscribe to a documented resource and receive at least one notification on change.
- `v15.0.0` tagged after retro.

## Out of scope

- Full MCP streamable HTTP spec.
- Replacing `/mcp/stream` workspace event SSE.
- SQLite semantic search.

## References

- Cluster B / H retros — `resources/subscribe` deferral.
- `maidan-mcp`, `GET /mcp/stream` in [[Architecture]].
