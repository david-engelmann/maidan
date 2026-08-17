-- Cluster 226 (Program B, agentic orchestration): scheduled / recurring task
-- foundation. A schedule materializes a task thread when it is due.
--
-- interval_secs NULL = one-shot (fires once, then active=false); a positive value
-- = recurring (re-arm next_run_at += interval_secs after each firing). A later
-- cluster's background sweeper creates a thread titled `title` in `channel_id`
-- when `active AND next_run_at <= now`. No routes/worker in this cluster — the
-- zero-blast-radius foundation pattern (Cluster 159 / 217).
CREATE TABLE IF NOT EXISTS maidan_task_schedules (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    interval_secs BIGINT,
    next_run_at TIMESTAMPTZ NOT NULL,
    last_run_at TIMESTAMPTZ,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (interval_secs IS NULL OR interval_secs > 0)
);

-- The sweeper's due-scan orders active schedules by next_run_at.
CREATE INDEX IF NOT EXISTS idx_task_schedules_due
    ON maidan_task_schedules (next_run_at);
