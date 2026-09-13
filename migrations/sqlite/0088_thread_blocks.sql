-- Cluster 386 (Wave 2 #27, G14 + W2): SQLite mirror of postgres 0089.
-- Explicit blocked-reason. Presence = blocked; absence = unblocked.
-- Numbered 0088: Cluster 385 Soundcheck took sqlite 0087.
CREATE TABLE maidan_thread_blocks (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN ('dag', 'gate', 'human', 'child', 'quota', 'unclaimable')),
    set_by TEXT NOT NULL REFERENCES maidan_members(id),
    set_at TEXT NOT NULL DEFAULT (datetime('now'))
);
