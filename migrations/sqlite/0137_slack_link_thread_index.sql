-- Egress resolves a Slack link by the Maidan thread. See the Postgres twin.
CREATE INDEX IF NOT EXISTS idx_slack_links_thread
    ON maidan_slack_channel_links (thread_id);
