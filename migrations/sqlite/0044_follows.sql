-- Cluster 244 (mirror of postgres 0045): subscription/follows — a member follows a
-- channel or thread to be notified of activity there (presence of a row = following).
-- The reverse indexes give the router the follower set per channel/thread.
-- Zero-blast-radius foundation — no router change or routes yet.
CREATE TABLE IF NOT EXISTS maidan_channel_follows (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, channel_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_follows_channel ON maidan_channel_follows (channel_id);

CREATE TABLE IF NOT EXISTS maidan_thread_follows (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, thread_id)
);
CREATE INDEX IF NOT EXISTS idx_thread_follows_thread ON maidan_thread_follows (thread_id);
