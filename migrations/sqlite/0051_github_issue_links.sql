-- GitHub projector issue/PR links (Cluster 311). SQLite mirror.
CREATE TABLE IF NOT EXISTS maidan_github_issue_links (
    repo TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo, issue_number)
);

CREATE INDEX IF NOT EXISTS idx_github_links_workspace
    ON maidan_github_issue_links (workspace_id);
CREATE INDEX IF NOT EXISTS idx_github_links_thread
    ON maidan_github_issue_links (thread_id);
