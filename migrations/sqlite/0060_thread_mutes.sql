-- Cluster 356 (F7, mirror of postgres 0061): leaf mute — a member mutes a
-- specific thread so the notification router suppresses notifications about it.
-- Presence of a row = muted.
CREATE TABLE IF NOT EXISTS maidan_thread_mutes (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, thread_id)
);
