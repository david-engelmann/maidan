-- Cluster 366 (Wave 1 #14, N1): Web Push subscriptions (SQLite twin of pg 0071).
-- One row per member device; the router delivers a Web Push message here when the
-- member has no live WebSocket connection.
CREATE TABLE IF NOT EXISTS maidan_push_subscriptions (
    id          TEXT PRIMARY KEY,
    member_id   TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    endpoint    TEXT NOT NULL,
    p256dh      TEXT NOT NULL,
    auth        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    UNIQUE (member_id, endpoint)
);
CREATE INDEX IF NOT EXISTS idx_push_subscriptions_member
    ON maidan_push_subscriptions (member_id);
