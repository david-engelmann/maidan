-- Cluster 252 (mirror of postgres 0047): a durable per-member last-seen timestamp,
-- a cross-replica signal for presence-aware email routing (presence is otherwise
-- in-memory + per-replica). One row per member, upserted on presence registration.
-- Zero-blast-radius foundation — no wiring yet.
CREATE TABLE IF NOT EXISTS maidan_member_last_seen (
    member_id TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now'))
);
