# Cluster 86.0 retro — Per-model embeddings query surface

> Tag **`v86.0.0`**.

## What shipped

- Optional `embedding_model` on `GET /workspaces/:wid/search` (semantic) and MCP `search_messages`.
- Defaults to active embedding provider `model_name()`; queries the matching per-model table (registry + `maidan_emb_*` from Cluster 47).
- `sqlite_http_semantic_search_honors_embedding_model_param` e2e; [[Production]] query param table.

## What was deferred

- Automatic reindex when provider env changes (use CLI or Cluster **87** job API).
- MCP/HTTP query vectors from a non-active provider (embed step always uses configured provider).

## Next

Cluster **87** — reindex job API ([[Clusters/Product Ladder 77+]]).
