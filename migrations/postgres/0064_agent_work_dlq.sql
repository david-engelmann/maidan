-- Cluster 358 (T1/T5): the agent-work dead-letter queue. When a claimed run is
-- stopped for exceeding its budget envelope (Cluster 358.3), the claim fails and
-- a row is recorded here — so the failed work is triageable rather than silently
-- lost or marked done. A hard stop is NOT success. `reason` is a BudgetReason
-- (tokens|usd|turns|wall); usage columns snapshot consumption at failure.
CREATE TABLE IF NOT EXISTS maidan_agent_work_dlq (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    used_tokens BIGINT NOT NULL DEFAULT 0,
    used_usd_micros BIGINT NOT NULL DEFAULT 0,
    used_turns BIGINT NOT NULL DEFAULT 0,
    failed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_agent_work_dlq_channel
    ON maidan_agent_work_dlq (channel_id, failed_at DESC);
