-- Cluster 411: durable, reviewable authority for delegated personal-state actions.
CREATE TABLE IF NOT EXISTS maidan_delegation_grants (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    subject_id UUID NOT NULL REFERENCES maidan_members(id),
    delegate_id UUID NOT NULL REFERENCES maidan_members(id),
    capabilities TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (length(trim(purpose)) BETWEEN 1 AND 1000),
    authorized_by UUID NOT NULL REFERENCES maidan_members(id),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (subject_id <> delegate_id)
);

CREATE INDEX IF NOT EXISTS idx_delegation_grants_workspace_created
    ON maidan_delegation_grants (workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_delegation_grants_delegate_active
    ON maidan_delegation_grants (workspace_id, delegate_id, expires_at)
    WHERE revoked_at IS NULL;
