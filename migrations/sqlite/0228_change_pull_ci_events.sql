-- CI read from GitHub's events, not polled; see the Postgres twin (0228).
ALTER TABLE maidan_change_pulls ADD COLUMN ci_event_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN ci_read_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN ci_read_head TEXT;
CREATE INDEX IF NOT EXISTS maidan_change_pulls_ci_read_head
    ON maidan_change_pulls (repo, ci_read_head) WHERE closed_at IS NULL;
