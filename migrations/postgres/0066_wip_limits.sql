-- Cluster 362 (G11): per-workspace work-in-progress limit. A row caps the number
-- of concurrent *live* claims (assigned, non-terminal, unexpired-lease) any single
-- member may hold in the workspace; no row means unlimited. `wip_limit = 0` freezes
-- claiming entirely. Opt-in: unset until an admin configures it.
CREATE TABLE maidan_wip_limits (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    wip_limit BIGINT NOT NULL CHECK (wip_limit >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
