-- Per-workspace usage meters; see the Postgres twin (0197).
CREATE TABLE maidan_usage_meters (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    day          TEXT NOT NULL,
    meter        TEXT NOT NULL,
    quantity     INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, day, meter)
);
