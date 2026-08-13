-- Cluster 204: per-workspace artifact access links.
--
-- Artifacts are content-addressed and deduped across workspaces (no
-- `workspace_id` on `maidan_artifacts`), so a caller who knows a SHA-256 could
-- fetch any tenant's blob with only `workspace:read`. This table records which
-- workspaces may access each SHA; `get_artifact*` enforces a matching row.
CREATE TABLE IF NOT EXISTS maidan_artifact_refs (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, sha256)
);

CREATE INDEX IF NOT EXISTS idx_artifact_refs_sha ON maidan_artifact_refs (sha256);

-- Backfill: link each existing artifact to its uploader's workspace so existing
-- workspaces keep access to what they uploaded.
INSERT INTO maidan_artifact_refs (workspace_id, sha256)
SELECT DISTINCT m.workspace_id, a.sha256
FROM maidan_artifacts a
JOIN maidan_members m ON m.id = a.uploaded_by
ON CONFLICT DO NOTHING;
