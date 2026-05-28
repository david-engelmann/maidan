# Cluster 28.0 retro — Privacy complete

> Closing wave for Cluster 28.0 · target tag **`v28.0.0`**.

Cluster 28.0 deepens Cluster 25 message purge and adds operator audit visibility.

## What shipped

- Deep `purge_workspace_messages`: references, embeddings count, API token revocation,
  workspace event log deletion (SQLite + Postgres).
- `GET /workspaces/:id/audit` with workspace-scoped filter.
- Tests: `workspace_purge_deep`, `workspace_purge_deep_postgres`, `workspace_audit_e2e`.

## What was deferred

| To | What | Why |
|----|------|-----|
| [[Remaining Work]] | Artifact blob deletion, workspace row delete | Content-addressed store + FK policy |
| Post-28 | Purge UI tab | API-first |
| Post-28 | `mcp-stdio` Postgres | Separate transport cluster |

## Surprises

- Embeddings CASCADE on message delete; count taken before purge for accurate reporting.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| Deep workspace purge counts | `v28.0.0` |
| Workspace audit HTTP list | `v28.0.0` |

## Forward look

Pick next epic from [[Remaining Work]]: MCP session mux, message edit, Helm umbrella, or rate limits.

## Acknowledgements

- Maintainer-driven cluster 28 implementation.
