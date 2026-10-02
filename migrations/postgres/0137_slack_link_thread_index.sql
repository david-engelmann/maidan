-- Egress resolves a Slack link by the Maidan thread (`get_by_thread`).
-- The table was indexed on workspace_id only, so that lookup scanned
-- every link. An index is safe while the previous binary is still
-- serving: it does not change a column the old binary reads or writes.
CREATE INDEX IF NOT EXISTS idx_slack_links_thread
    ON maidan_slack_channel_links (thread_id);
