CREATE TABLE maidan_a2a_push_configs (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    push_url TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE maidan_a2a_tasks (
    id TEXT PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    task_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_maidan_a2a_tasks_workspace ON maidan_a2a_tasks (workspace_id);
