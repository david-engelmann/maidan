-- Cluster 362 (G11): per-workspace work-in-progress limit (SQLite mirror of
-- postgres 0066). A row caps the concurrent *live* claims a single member may hold
-- in the workspace; no row = unlimited; wip_limit = 0 freezes claiming.
CREATE TABLE maidan_wip_limits (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    wip_limit INTEGER NOT NULL CHECK (wip_limit >= 0),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
