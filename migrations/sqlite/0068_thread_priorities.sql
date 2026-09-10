-- Cluster 365 (G3 fair dispatch): per-thread dispatch priority (SQLite twin of
-- pg 0069). Absence = default priority 0; higher = more urgent. `claim_next`
-- orders by an aged effective rank so priority jumps the queue without starving
-- long-waiting normal tasks.
CREATE TABLE IF NOT EXISTS maidan_thread_priorities (
    thread_id  TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    priority   INTEGER NOT NULL DEFAULT 0,
    set_by     TEXT NOT NULL REFERENCES maidan_members(id),
    set_at     TEXT NOT NULL
);
