-- Retry-then-disable for projector egress (Cluster 377.3). A delivery that fails
-- with an auth/config-class error — GitHub 401/403/404, Slack `invalid_auth` /
-- `channel_not_found` — cannot be fixed by retrying: the token is wrong, the scope
-- was revoked, the channel is gone. The link is turned off here so every later
-- message into it stops queueing eight doomed attempts, and a
-- `ProjectorMisconfigured` event says so loudly.
--
-- NULL = enabled (so every existing link stays on). Re-linking the channel/issue
-- clears it — the link upsert resets `disabled_at`, which is the re-enable path.
ALTER TABLE maidan_slack_channel_links ADD COLUMN IF NOT EXISTS disabled_at TIMESTAMPTZ;
ALTER TABLE maidan_github_issue_links ADD COLUMN IF NOT EXISTS disabled_at TIMESTAMPTZ;
