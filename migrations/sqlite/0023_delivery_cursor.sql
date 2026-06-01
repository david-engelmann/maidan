-- Per-consumer delivery cursors (Cluster 56.0), SQLite parity with Postgres 0015.

CREATE TABLE maidan_delivery_cursor (
    consumer_id            TEXT NOT NULL,
    workspace_id           TEXT NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    last_delivered_log_id  INTEGER NOT NULL DEFAULT 0,
    updated_at             TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (consumer_id, workspace_id)
);

CREATE INDEX idx_delivery_cursor_workspace ON maidan_delivery_cursor (workspace_id);
