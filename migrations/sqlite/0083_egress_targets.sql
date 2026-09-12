-- The egress trust boundary (Cluster 378.1; see postgres/0084). SQLite mirror:
-- timestamps are rfc3339 text bound by the store.
CREATE TABLE IF NOT EXISTS maidan_egress_targets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL, -- a Slack channel id, or a GitHub `owner/name`
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_egress_targets_unique
    ON maidan_egress_targets (workspace_id, surface, selector);
