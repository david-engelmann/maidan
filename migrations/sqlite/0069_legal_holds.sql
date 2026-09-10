-- Cluster 366 (Wave 1 #14, T6): legal hold (SQLite twin of pg 0070). A held
-- workspace is protected from retention pruning (its events survive; audit prune
-- freezes while any hold exists) and from purge/erase. Presence = under hold.
CREATE TABLE IF NOT EXISTS maidan_legal_holds (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    placed_by    TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    placed_at    TEXT NOT NULL
);
