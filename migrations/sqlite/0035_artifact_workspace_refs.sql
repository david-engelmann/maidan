-- Cluster 204: per-workspace artifact access links (mirror of postgres 0036).
--
-- Artifacts are content-addressed + deduped across workspaces, so a known SHA
-- + `workspace:read` could fetch any tenant's blob. This table records which
-- workspaces may access each SHA; `get_artifact*` enforces a matching row.
CREATE TABLE IF NOT EXISTS maidan_artifact_refs (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (workspace_id, sha256)
);

CREATE INDEX IF NOT EXISTS idx_artifact_refs_sha ON maidan_artifact_refs (sha256);

-- Backfill from the uploader's workspace (see the postgres migration).
INSERT OR IGNORE INTO maidan_artifact_refs (workspace_id, sha256)
SELECT DISTINCT m.workspace_id, a.sha256
FROM maidan_artifacts a
JOIN maidan_members m ON m.id = a.uploaded_by;
