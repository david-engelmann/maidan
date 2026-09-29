# Product completion checklist (Cluster 58 / Maidan 2.0 gate)

Critical-path surfaces for agents and operators after Clusters 17–57.

| Area | Status | Notes |
|------|--------|-------|
| HTTP CRUD + auth | ✓ | Capability matrix e2e |
| MCP `POST /mcp` | ✓ | tools/resources/prompts |
| MCP streamable HTTP | ✓ | `POST /mcp/streamable` |
| MCP notifications SSE | ✓ | `GET /mcp/notifications` (v16 compat) |
| A2A `SendMessage` / `GetTask` | ✓ | `POST /a2a/v1/rpc` |
| Search lexical + semantic | ✓ | SQLite + Postgres (`pgvector`) |
| S3 multipart artifacts | ✓ | Cluster 19 |
| Message router | ✓ | `maidan-router` |
| Web UI `/ui` | ✓ | events, search, FSM, tokens |
| Helm deploy | ✓ | `helm/maidan`, stack bundle |
| Helm kind install CI | ✓ | `scripts/helm-install-kind-smoke.sh` (Cluster 55) |
| Workspace purge (deep) | ✓ | messages, embeddings, refs, tokens, events |
| Workspace full erasure | ✓ | `DELETE /workspaces/:id` + confirm (Cluster 53) |
| Federation ingest | ✓ | peer bearer |
| DMs | ✓ | Cluster 39 |
| Message edit history | ✓ | Cluster 46 |
| Outbound webhooks | ✓ | HMAC delivery, thread-close events (Cluster 50) |
| FSM automation hooks | ✓ | Cluster 52 |
| Slash commands | ✓ | Cluster 51 |
| Per-token capability quotas | ✓ | Cluster 54 |
| Delivery cursor + outbox replay | ✓ | Cluster 56 |
| Installed agent apps | ✓ | scoped app tokens (Cluster 57) |
| OpenAPI + metrics | ✓ | `/openapi.json`, `/metrics` |

Deferred (not blocking 2.0): OTLP dashboards, multi-region HA, OAuth app install UI, in-cluster cert-manager install.

Validation:

- `cargo test -p maidan-server --test product_completion_gate_e2e`
- `./scripts/helm-template-smoke.sh`
- `./scripts/helm-install-kind-smoke.sh` (CI `helm install (kind)`)
- docker compose smoke
