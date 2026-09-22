-- Cluster 404: follow another member's work occupancy. Presence remains
-- ephemeral; this table stores only the durable subscription edge.
CREATE TABLE IF NOT EXISTS maidan_member_follows (
    follower_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    followed_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (follower_id, followed_id),
    CHECK (follower_id <> followed_id)
);
CREATE INDEX IF NOT EXISTS idx_member_follows_followed
    ON maidan_member_follows (followed_id);
