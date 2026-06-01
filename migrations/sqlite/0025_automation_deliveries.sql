-- Durable HTTP delivery for slash commands and FSM hooks (Cluster 68.0).

CREATE TABLE maidan_automation_deliveries (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id     TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    source_kind      TEXT NOT NULL,
    source_id        TEXT NOT NULL,
    target_url       TEXT NOT NULL,
    header_name      TEXT NOT NULL,
    header_value     TEXT NOT NULL,
    payload          TEXT NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_error       TEXT,
    delivered_at     TEXT,
    quarantined_at   TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_automation_deliveries_pending
    ON maidan_automation_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;

CREATE INDEX idx_automation_deliveries_workspace
    ON maidan_automation_deliveries (workspace_id, created_at DESC);
