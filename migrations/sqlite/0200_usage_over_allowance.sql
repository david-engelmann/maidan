-- Allowance crossings; see the Postgres twin (0200).
CREATE TABLE maidan_usage_over_allowance (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    meter        TEXT NOT NULL,
    month        TEXT NOT NULL,
    quantity     INTEGER NOT NULL,
    allowance    INTEGER NOT NULL,
    marked_at    TEXT NOT NULL,
    PRIMARY KEY (workspace_id, meter, month)
);
CREATE INDEX idx_usage_over_allowance_month ON maidan_usage_over_allowance (month);
