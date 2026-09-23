-- Cluster 411 (Postgres 0103 twin): durable delegated authority.
CREATE TABLE IF NOT EXISTS maidan_delegation_grants (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    subject_id TEXT NOT NULL REFERENCES maidan_members(id),
    delegate_id TEXT NOT NULL REFERENCES maidan_members(id),
    capabilities TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (length(trim(purpose)) BETWEEN 1 AND 1000),
    authorized_by TEXT NOT NULL REFERENCES maidan_members(id),
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (subject_id <> delegate_id)
);

CREATE INDEX IF NOT EXISTS idx_delegation_grants_workspace_created
    ON maidan_delegation_grants (workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_delegation_grants_delegate_active
    ON maidan_delegation_grants (workspace_id, delegate_id, expires_at)
    WHERE revoked_at IS NULL;
