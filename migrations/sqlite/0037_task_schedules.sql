-- Cluster 226 (mirror of postgres 0038): scheduled / recurring task foundation.
--
-- interval_secs NULL = one-shot (fires once, then active=0); a positive value =
-- recurring (re-arm next_run_at += interval_secs after each firing). A later
-- cluster's sweeper creates a thread titled `title` in `channel_id` when
-- `active AND next_run_at <= now`. No routes/worker yet (zero blast radius).
CREATE TABLE IF NOT EXISTS maidan_task_schedules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    interval_secs INTEGER,
    next_run_at TEXT NOT NULL,
    last_run_at TEXT,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    created_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (interval_secs IS NULL OR interval_secs > 0)
);

CREATE INDEX IF NOT EXISTS idx_task_schedules_due
    ON maidan_task_schedules (next_run_at);
