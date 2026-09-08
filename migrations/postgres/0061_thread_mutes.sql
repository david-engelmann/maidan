-- Cluster 356 (F7): leaf mute. A member mutes a SPECIFIC thread so the
-- notification router suppresses notifications about it, even if they follow its
-- channel or are otherwise a recipient. Distinct from channel-level or per-kind
-- mute (Cluster 242) — this is per-thread (per-leaf). Presence of a row = muted.
CREATE TABLE IF NOT EXISTS maidan_thread_mutes (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, thread_id)
);
