-- Cluster 237 (mirror of postgres 0042): per-recipient notifications — one row
-- per (recipient, source event) with denormalized context + per-recipient read
-- state (`read_at` NULL = unread). Zero-blast-radius foundation — no
-- router/routes/worker yet.
CREATE TABLE IF NOT EXISTS maidan_notifications (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    source_log_id INTEGER NOT NULL,
    channel_id TEXT,
    thread_id TEXT,
    message_id TEXT,
    actor_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    read_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_notifications_member
    ON maidan_notifications (member_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_member_unread
    ON maidan_notifications (member_id) WHERE read_at IS NULL;
