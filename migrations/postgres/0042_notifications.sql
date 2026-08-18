-- Cluster 237 (Program C, Arc G): per-recipient notifications. Where a mention
-- is one shared `maidan_mentions` row read through a single inbox cursor, this
-- is one row per (recipient, source event) — who should know, what triggered it
-- (`kind` = the source EventKind, `source_log_id` = the maidan_events row),
-- denormalized context for rendering, and per-recipient read state (`read_at`
-- NULL = unread). The zero-blast-radius foundation (Cluster 159 / 217 / 226 /
-- 230 / 234) for the notification router + unified inbox that follow — no
-- router/routes/worker in this cluster.
CREATE TABLE IF NOT EXISTS maidan_notifications (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    source_log_id BIGINT NOT NULL,
    channel_id UUID,
    thread_id UUID,
    message_id UUID,
    actor_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_notifications_member
    ON maidan_notifications (member_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_member_unread
    ON maidan_notifications (member_id) WHERE read_at IS NULL;
