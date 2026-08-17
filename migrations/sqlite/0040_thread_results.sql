-- Cluster 234 (mirror of postgres 0041): a task's structured result (JSON stored
-- as TEXT in SQLite). One result per thread (upsert). Zero-blast-radius
-- foundation — no routes/worker yet.
CREATE TABLE IF NOT EXISTS maidan_thread_results (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    result TEXT NOT NULL,
    produced_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    produced_at TEXT NOT NULL DEFAULT (datetime('now'))
);
