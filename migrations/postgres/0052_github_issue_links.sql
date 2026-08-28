-- GitHub projector issue/PR links (Cluster 311): maps a GitHub issue/PR
-- (repo full-name + number) to the Maidan channel/thread it projects into, and the
-- member inbound comments are posted as. One Maidan thread per GitHub issue/PR.
CREATE TABLE IF NOT EXISTS maidan_github_issue_links (
    repo TEXT NOT NULL,
    issue_number BIGINT NOT NULL,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo, issue_number)
);

CREATE INDEX IF NOT EXISTS idx_github_links_workspace
    ON maidan_github_issue_links (workspace_id);
CREATE INDEX IF NOT EXISTS idx_github_links_thread
    ON maidan_github_issue_links (thread_id);
