# Product completion checklist (Cluster 26)

Critical-path surfaces for agents and operators after Clusters 17–27.

| Area | Status | Notes |
|------|--------|-------|
| HTTP CRUD + auth | ✓ | Capability matrix e2e |
| MCP `POST /mcp` | ✓ | tools/resources/prompts |
| MCP streamable HTTP | ✓ | `POST /mcp/streamable` |
| MCP notifications SSE | ✓ | `GET /mcp/notifications` (v16 compat) |
| A2A `SendMessage` / `GetTask` | ✓ | `POST /a2a/v1/rpc` |
| Search lexical + semantic | ✓ | SQLite + Postgres |
| S3 multipart artifacts | ✓ | Cluster 19 |
| Message router | ✓ | `maidan-router` |
| Web UI `/ui` | ✓ | events, search, FSM, tokens |
| Helm deploy | ✓ | `helm/maidan` |
| Workspace purge (deep) | ✓ | Purge messages, embeddings, references, tokens, events; GET audit |
| Federation ingest | ✓ | peer bearer |

Deferred (not blocking 27): OTLP dashboards (Cluster T), multi-region HA.

Validation: `cargo test -p maidan-server --test product_completion_gate_e2e`, `./scripts/helm-template-smoke.sh`, docker compose smoke.
