# Cluster 25.0 — Privacy & erasure

> **Goal:** Workspace purge API with audit trail for tombstone/purge operations.
>
> **Target tag:** `v25.0.0`.

## Exit criteria

- `POST /workspaces/:id/purge` tombstones then hard-deletes all workspace messages.
- `workspace.purge` row in `maidan_audit_events`.
- `v25.0.0` tagged after retro.

## References

- [[Clusters/Product Ladder 17-27]], store tests `workspace_purge.rs`.
