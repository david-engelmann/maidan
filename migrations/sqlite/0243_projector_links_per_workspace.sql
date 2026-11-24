-- A Slack channel link and a GitHub issue link belong to the workspace that
-- made them. See the Postgres twin. SQLite cannot change a primary key, so
-- each table is rebuilt.
CREATE TABLE maidan_slack_channel_links_per_workspace (
    slack_channel_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    disabled_at TEXT,
    PRIMARY KEY (workspace_id, slack_channel_id)
);
INSERT INTO maidan_slack_channel_links_per_workspace
    (slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at)
    SELECT slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at
    FROM maidan_slack_channel_links;
DROP TABLE maidan_slack_channel_links;
ALTER TABLE maidan_slack_channel_links_per_workspace RENAME TO maidan_slack_channel_links;
CREATE INDEX idx_slack_links_workspace ON maidan_slack_channel_links (workspace_id);
CREATE INDEX idx_slack_links_thread ON maidan_slack_channel_links (thread_id);
CREATE INDEX idx_slack_links_channel ON maidan_slack_channel_links (slack_channel_id);

CREATE TABLE maidan_github_issue_links_per_workspace (
    repo TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    disabled_at TEXT,
    PRIMARY KEY (workspace_id, repo, issue_number)
);
INSERT INTO maidan_github_issue_links_per_workspace
    (repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at)
    SELECT repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at
    FROM maidan_github_issue_links;
DROP TABLE maidan_github_issue_links;
ALTER TABLE maidan_github_issue_links_per_workspace RENAME TO maidan_github_issue_links;
CREATE INDEX idx_github_links_workspace ON maidan_github_issue_links (workspace_id);
CREATE UNIQUE INDEX idx_github_links_thread ON maidan_github_issue_links (thread_id);
CREATE INDEX idx_github_links_issue ON maidan_github_issue_links (repo, issue_number);
