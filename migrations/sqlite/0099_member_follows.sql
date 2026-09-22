-- Cluster 404 (Postgres 0100 twin): durable member-occupancy subscriptions.
-- Presence remains ephemeral and outside the event log/database.
CREATE TABLE IF NOT EXISTS maidan_member_follows (
    follower_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    followed_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (follower_id, followed_id),
    CHECK (follower_id <> followed_id)
);
CREATE INDEX IF NOT EXISTS idx_member_follows_followed
    ON maidan_member_follows (followed_id);
