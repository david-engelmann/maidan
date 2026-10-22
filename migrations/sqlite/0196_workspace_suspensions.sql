-- Suspended workspaces; see the Postgres twin (0196).
CREATE TABLE maidan_workspace_suspensions (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    suspended_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    suspended_at TEXT NOT NULL
);
