-- Idempotency keys for retried writes. See the Postgres twin. Times are
-- millisecond `...Z` text, compared as strings.
CREATE TABLE IF NOT EXISTS maidan_idempotency_keys (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    status INTEGER,
    content_type TEXT,
    body BLOB,
    locked_until TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (workspace_id, actor_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS maidan_idempotency_keys_expires_at
    ON maidan_idempotency_keys (expires_at);
