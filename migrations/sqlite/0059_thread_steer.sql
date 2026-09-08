-- Cluster 355 (W1, mirror of postgres 0060): persisted steering guidance for a
-- task/thread. One steer per thread (upsert, latest wins); survives claims and
-- handoffs. Distinct from a Cluster-195 handoff note (event-borne, not persisted).
CREATE TABLE IF NOT EXISTS maidan_thread_steer (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    steer TEXT NOT NULL,
    steered_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    steered_at TEXT NOT NULL DEFAULT (datetime('now'))
);
