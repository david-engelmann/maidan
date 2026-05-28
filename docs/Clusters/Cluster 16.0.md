# Cluster 16.0 — MCP streamable HTTP parity

Cluster 15.0 closed stdio MCP resource subscriptions at **`v15.0.0`**. `POST /mcp`
is still request/response-only, so HTTP MCP clients cannot receive
`resources/updated` notifications without polling.

> **Goal:** Add streamable HTTP transport for MCP resource subscription
> notifications so HTTP clients can receive `notifications/resources/updated`
> parity with stdio.
>
> **Target tag:** `v16.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 16.0 kickoff plan`                                      | —     |
| 16.0.1     | `feat(maidan-server): MCP streamable subscribe endpoint`               | TBD   |
| 16.0.2     | `feat(maidan-mcp): transport bridge for notification fan-out`          | TBD   |
| 16.0.3     | `test: HTTP MCP subscribe integration`                                 | TBD   |
| 16.0.4     | `docs: streamable MCP subscribe in Architecture + Production`          | TBD   |
| 16.0.retro | `docs(retro): Cluster 16.0 + v16.0.0 tag prep`                         | TBD   |

## Order

1. **16.0.1** — add HTTP streaming endpoint shape and lifecycle.
2. **16.0.2** — bridge subscription events from dispatcher to transport stream.
3. **16.0.3** — integration tests for subscribe, event delivery, and disconnect cleanup.
4. **16.0.4** — docs and MCP reference update.
5. **16.0.retro** + `v16.0.0` tag.

## Exit criteria

- CI green on `main`.
- HTTP MCP client receives at least one `notifications/resources/updated`
  message after subscribing and triggering a mutation.
- `v16.0.0` tagged after retro.

## Out of scope

- Non-resource event streaming replacement for `/mcp/stream`.
- SQLite semantic search.
- S3 multipart uploads.

## References

- [[Clusters/Cluster 15.0]], [[Retros/Cluster 15.0]].
