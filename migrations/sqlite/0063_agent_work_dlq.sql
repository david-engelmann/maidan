-- Cluster 358 (T1/T5, mirror of postgres 0064): the agent-work dead-letter queue.
-- A stopped-for-budget run is recorded here (reason = tokens|usd|turns|wall) so
-- the failed work is triageable. A hard stop is NOT success.
CREATE TABLE IF NOT EXISTS maidan_agent_work_dlq (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    used_tokens INTEGER NOT NULL DEFAULT 0,
    used_usd_micros INTEGER NOT NULL DEFAULT 0,
    used_turns INTEGER NOT NULL DEFAULT 0,
    failed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_agent_work_dlq_channel
    ON maidan_agent_work_dlq (channel_id, failed_at DESC);
