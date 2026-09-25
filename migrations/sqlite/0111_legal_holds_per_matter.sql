-- One row per matter; see the Postgres twin (0112). SQLite cannot change a
-- primary key in place, so the table is rebuilt; nothing references it.
CREATE TABLE maidan_legal_holds_per_matter (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    placed_by    TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    placed_at    TEXT NOT NULL
);
INSERT INTO maidan_legal_holds_per_matter (id, workspace_id, reason, placed_by, placed_at)
    SELECT randomblob(16), workspace_id, reason, placed_by, placed_at FROM maidan_legal_holds;
DROP TABLE maidan_legal_holds;
ALTER TABLE maidan_legal_holds_per_matter RENAME TO maidan_legal_holds;
CREATE INDEX idx_legal_holds_workspace ON maidan_legal_holds (workspace_id, placed_at);
