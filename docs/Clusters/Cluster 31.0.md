# Cluster 31.0 — Workspace artifact purge

**Theme:** Deep workspace purge removes artifact metadata and content-addressed blobs.

## Scope

- `WorkspacePurgeResult.artifacts_removed`; blob `delete` via `ArtifactStore` in `POST /workspaces/:id/purge`.
- Store: delete `maidan_artifacts` rows for workspace members (Postgres + SQLite).

## Tag

`v31.0.0`

## Tests

- `workspace_purge_deep` (artifact row)
- `workspace_purge_artifact_blobs_e2e` (HTTP + LocalFs)
