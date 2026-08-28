-- Slack projector channel links (Cluster 308). SQLite mirror.
CREATE TABLE IF NOT EXISTS maidan_slack_channel_links (
    slack_channel_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_slack_links_workspace
    ON maidan_slack_channel_links (workspace_id);
