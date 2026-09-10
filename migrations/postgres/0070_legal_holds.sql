-- Cluster 366 (Wave 1 #14, T6): legal hold. A workspace under legal hold is
-- protected from deletion: retention pruning skips its event-log rows, audit
-- pruning is frozen while any hold is active (maidan_audit is not workspace-
-- tagged), and workspace purge/erase is refused (409). Presence of a row =
-- under hold; lifting = deleting the row. One active hold per workspace.
CREATE TABLE IF NOT EXISTS maidan_legal_holds (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    placed_by    UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    placed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
