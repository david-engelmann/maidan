-- A Slack channel link and a GitHub issue link belong to the workspace that
-- made them. They were unique across the whole server, so the first workspace
-- to link a channel or an issue held it against every other, and a refused
-- link told one tenant that another had linked it. Each workspace now links
-- the same channel or issue for itself, and an inbound delivery goes to the
-- links of the workspaces its route serves. One link per thread stays.
ALTER TABLE maidan_slack_channel_links DROP CONSTRAINT maidan_slack_channel_links_pkey;
ALTER TABLE maidan_slack_channel_links ADD PRIMARY KEY (workspace_id, slack_channel_id);
CREATE INDEX IF NOT EXISTS idx_slack_links_channel
    ON maidan_slack_channel_links (slack_channel_id);

ALTER TABLE maidan_github_issue_links DROP CONSTRAINT maidan_github_issue_links_pkey;
ALTER TABLE maidan_github_issue_links ADD PRIMARY KEY (workspace_id, repo, issue_number);
CREATE INDEX IF NOT EXISTS idx_github_links_issue
    ON maidan_github_issue_links (repo, issue_number);
