# Cluster 53.0 retro — Workspace full erasure

> Tag **`v53.0.0`**.

## What shipped

- `Store::erase_workspace`: runs deep purge then deletes `maidan_workspaces` (CASCADE removes members, channels, peers, hooks, webhooks, OIDC rows, etc.).
- `DELETE /workspaces/:id` with `confirm_workspace_id` confirmation body.
- Pre-delete audit row `workspace.erase`; artifact blob cleanup mirrors purge.

## What was deferred

- UI “delete workspace” tab (purge UI exists).
- Cross-workspace audit aggregation for erased workspace ids.
- MCP `erase_workspace` tool.

## Forward look

Cluster **54**: capability quotas and distributed rate limits.
