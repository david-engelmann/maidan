-- A2A v1.0 per-task push notification configs (Cluster 284).
-- The spec models push configs per-task with a stable config id (Create/Get/List/
-- Delete), unlike the earlier one-per-workspace `maidan_a2a_push_configs` table.
CREATE TABLE IF NOT EXISTS maidan_a2a_task_push_configs (
    task_id    TEXT NOT NULL,
    config_id  TEXT NOT NULL,
    push_url   TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (task_id, config_id)
);

CREATE INDEX IF NOT EXISTS idx_a2a_task_push_configs_task
    ON maidan_a2a_task_push_configs (task_id);
