-- Cluster 357 (N3): per-channel mute. A member mutes a whole channel so the
-- notification router suppresses its firehose (MessagePosted follow
-- notifications), even if they follow the channel. Distinct from per-thread
-- (Cluster 356) and per-kind (Cluster 242) mute — this is per-channel. A
-- MentionRecorded breaks through a channel mute (357.2 router policy). Presence
-- of a row = muted.
CREATE TABLE IF NOT EXISTS maidan_channel_mutes (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, channel_id)
);
