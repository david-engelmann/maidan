CREATE TABLE maidan_a2a_push_configs (
    workspace_id TEXT NOT NULL PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    push_url TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE maidan_a2a_tasks (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    task_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_maidan_a2a_tasks_workspace ON maidan_a2a_tasks (workspace_id);
