# Cluster 31.0 retro — Workspace artifact purge

> Tag **`v31.0.0`**.

## What shipped

- Deep purge deletes artifact metadata for workspace members and best-effort blob removal.
- `artifacts_removed` on `WorkspacePurgeResult`; audit includes `artifact_blobs_deleted`.

## What was deferred

- Artifacts not tied to `uploaded_by` (orphan SHA rows).
- Workspace row / member / channel deletion.

## Forward look

Cluster 32: Helm umbrella; 33: MCP resource fan-out on HTTP mutations; 34: streamable session ids.
