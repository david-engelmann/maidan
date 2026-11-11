-- One delivery per event, however many replicas see it; see the Postgres twin
-- (0234). SQLite's tables are not partitioned, so the keys are the plain
-- (subscription, event) and (source, event) pairs.

DELETE FROM maidan_webhook_deliveries
WHERE EXISTS (
    SELECT 1 FROM maidan_webhook_deliveries keep
    WHERE keep.subscription_id = maidan_webhook_deliveries.subscription_id
      AND keep.log_id = maidan_webhook_deliveries.log_id
      AND keep.id < maidan_webhook_deliveries.id
);

CREATE UNIQUE INDEX idx_webhook_deliveries_event
    ON maidan_webhook_deliveries (subscription_id, log_id);

ALTER TABLE maidan_automation_deliveries ADD COLUMN log_id INTEGER;

CREATE UNIQUE INDEX idx_automation_deliveries_event
    ON maidan_automation_deliveries (source_kind, source_id, log_id);

CREATE TABLE maidan_fsm_hook_runs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    hook_id       TEXT NOT NULL REFERENCES maidan_fsm_hooks(id) ON DELETE CASCADE,
    log_id        INTEGER NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (hook_id, log_id)
);

CREATE INDEX idx_fsm_hook_runs_workspace
    ON maidan_fsm_hook_runs (workspace_id, created_at);
