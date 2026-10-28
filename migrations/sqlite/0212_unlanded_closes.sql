-- SQLite mirror of postgres 0213. A thread closed by close-abandoned
-- reached `closed` without landing; a row here keeps its dependents
-- waiting. Presence = closed without landing.
CREATE TABLE maidan_thread_unlanded_closes (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN ('duplicate', 'abandoned', 'superseded')),
    closed_by TEXT NOT NULL REFERENCES maidan_members(id),
    closed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
