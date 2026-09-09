-- Cluster 363 (G3): park a thread from dispatch (SQLite mirror of postgres 0067).
-- A row marks the thread "unclaimable" with a reason — `claim_next` skips it and
-- an explicit `claim` is refused — until cleared.
CREATE TABLE maidan_thread_unclaimable (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    marked_by TEXT NOT NULL REFERENCES maidan_members(id),
    marked_at TEXT NOT NULL DEFAULT (datetime('now'))
);
