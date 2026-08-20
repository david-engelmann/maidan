-- Cluster 252 (Program C, Arc I): a durable per-member last-seen timestamp. Presence
-- is otherwise in-memory + per-replica (presence.rs), so the notification router
-- can't tell whether a recipient is currently active. This gives presence-aware
-- email routing (a later cluster) a cross-replica signal: "email only if the member
-- hasn't been seen recently." One row per member, upserted on presence registration.
-- Zero-blast-radius foundation — no wiring yet.
CREATE TABLE IF NOT EXISTS maidan_member_last_seen (
    member_id UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
