-- A workspace an operator has suspended (hosted abuse controls). While a row
-- exists every credential of the workspace is refused and its outbound and
-- background work is skipped; its data is kept. Lifting deletes the row; the
-- audit log keeps both acts.
CREATE TABLE maidan_workspace_suspensions (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    suspended_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    suspended_at TIMESTAMPTZ NOT NULL
);
