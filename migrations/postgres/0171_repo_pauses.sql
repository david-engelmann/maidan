-- Repository pauses (maidan_types::footprint::RepoPause). While a repository
-- is paused in a workspace, the claim queue defers every thread whose
-- footprint names it, as it defers a thread that overlaps a live claim. The
-- pause gates new claims only; it never touches a claim already held.
CREATE TABLE maidan_repo_pauses (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    paused_at    TIMESTAMPTZ NOT NULL,
    paused_by    UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason       TEXT,
    PRIMARY KEY (workspace_id, repo)
);
