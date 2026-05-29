# Cluster 53.0 — Workspace full erasure

**Theme:** GDPR-style delete of the entire workspace shell, not only messages.

## Problem

`POST /workspaces/:id/purge` clears messages and revokes tokens but leaves
members, channels, threads, peers, and the workspace row.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `erase_workspace`: deep purge + `DELETE` workspace row (CASCADE) |
| HTTP | `DELETE /workspaces/:id` with `confirm_workspace_id` body |
| Audit | `workspace.erase` event recorded before deletion |
| Tests | Store + HTTP e2e |

## Tag

`v53.0.0`

See [[Clusters/Product Ladder 35+]] Phase VI.
