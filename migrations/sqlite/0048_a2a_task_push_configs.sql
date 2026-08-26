-- A2A v1.0 per-task push notification configs (Cluster 284). See the Postgres
-- 0049 migration for rationale.
CREATE TABLE IF NOT EXISTS maidan_a2a_task_push_configs (
    task_id    TEXT NOT NULL,
    config_id  TEXT NOT NULL,
    push_url   TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (task_id, config_id)
);

CREATE INDEX IF NOT EXISTS idx_a2a_task_push_configs_task
    ON maidan_a2a_task_push_configs (task_id);
