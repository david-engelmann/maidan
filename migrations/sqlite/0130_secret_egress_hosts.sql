-- The hosts each workspace trusts to receive its secret values (see
-- postgres/0130). SQLite mirror: timestamps are rfc3339 text bound by the store.
CREATE TABLE IF NOT EXISTS maidan_secret_egress_hosts (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (workspace_id, host)
);
