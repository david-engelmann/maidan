-- Retry-then-disable for projector egress (Cluster 377.3; see postgres/0083).
-- SQLite mirror: rfc3339 text, NULL = enabled.
ALTER TABLE maidan_slack_channel_links ADD COLUMN disabled_at TEXT;
ALTER TABLE maidan_github_issue_links ADD COLUMN disabled_at TEXT;
