-- Cluster 364 (G2/G4): wait-edges + on_timeout escalation (SQLite mirror of
-- postgres 0068). A durable timer on a thread; on timeout the escalation policy
-- fires (notify / park), never a decision. Cancelled (satisfied) or fired.
CREATE TABLE maidan_thread_waits (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    wait_until TEXT NOT NULL,
    on_timeout TEXT NOT NULL,
    reason TEXT,
    created_by TEXT NOT NULL REFERENCES maidan_members(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    fired_at TEXT
);

CREATE INDEX idx_thread_waits_due ON maidan_thread_waits (wait_until) WHERE fired_at IS NULL;
