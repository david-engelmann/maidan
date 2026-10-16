-- Orphaned artifact bytes waiting for another delete; see the Postgres twin
-- (0181). Times are millisecond `...Z` text, compared as strings against
-- `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`.
CREATE TABLE IF NOT EXISTS maidan_artifact_reap_retries (
    sha256 TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL,
    queued_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    next_attempt_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS maidan_artifact_reap_retries_due
    ON maidan_artifact_reap_retries (next_attempt_at);
