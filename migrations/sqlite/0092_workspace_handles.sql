-- Cluster 395 (mirror of postgres 0093): a renameable workspace handle
-- alias. Stored ids stay UUIDs; this row is the human name.
CREATE TABLE IF NOT EXISTS maidan_workspace_handles (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    handle TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (handle <> '')
);
