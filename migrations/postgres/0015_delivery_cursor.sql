-- Per-consumer delivery cursors (Cluster 13.0).

CREATE TABLE maidan_delivery_cursor (
    consumer_id            TEXT NOT NULL,
    workspace_id           UUID NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    last_delivered_log_id  BIGINT NOT NULL DEFAULT 0,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (consumer_id, workspace_id)
);

CREATE INDEX idx_delivery_cursor_workspace ON maidan_delivery_cursor (workspace_id);
