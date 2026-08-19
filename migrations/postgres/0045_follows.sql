-- Cluster 244 (Program C, Arc H): subscription/follows. A member follows a channel
-- or a thread to be notified of activity there even without a mention; the
-- notification router (a later cluster) reads the follower sets. Presence of a row =
-- following. The zero-blast-radius foundation (Cluster 230 pattern) — no router
-- change or routes in this cluster.
CREATE TABLE IF NOT EXISTS maidan_channel_follows (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, channel_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_follows_channel ON maidan_channel_follows (channel_id);

CREATE TABLE IF NOT EXISTS maidan_thread_follows (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, thread_id)
);
CREATE INDEX IF NOT EXISTS idx_thread_follows_thread ON maidan_thread_follows (thread_id);
