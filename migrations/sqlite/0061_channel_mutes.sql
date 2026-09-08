-- Cluster 357 (N3, mirror of postgres 0062): per-channel mute — a member mutes a
-- whole channel so the notification router suppresses its firehose. A mention
-- breaks through (357.2). Presence of a row = muted.
CREATE TABLE IF NOT EXISTS maidan_channel_mutes (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, channel_id)
);
