-- Cluster 395 (Wave 3 #35): a renameable workspace handle alias. The
-- workspace UUID is the stored id (room URI authority). A handle lives
-- on this table so row_to_workspace does not ripple. UNIQUE(handle)
-- is global — one alias per instance.
CREATE TABLE IF NOT EXISTS maidan_workspace_handles (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT maidan_workspace_handles_handle_nonempty CHECK (handle <> ''),
    CONSTRAINT maidan_workspace_handles_handle_unique UNIQUE (handle)
);
