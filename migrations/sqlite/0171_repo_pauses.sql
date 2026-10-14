-- Repository pauses; see the Postgres twin (0171).
CREATE TABLE maidan_repo_pauses (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    paused_at    TEXT NOT NULL,
    paused_by    TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason       TEXT,
    PRIMARY KEY (workspace_id, repo)
);
