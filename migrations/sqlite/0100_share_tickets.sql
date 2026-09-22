-- Cluster 405 (Postgres 0101 twin): time-boxed, read-only capability tickets.
CREATE TABLE IF NOT EXISTS maidan_share_tickets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES maidan_members(id),
    created_by TEXT NOT NULL REFERENCES maidan_members(id),
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (datetime(expires_at) > datetime(created_at))
);

CREATE INDEX IF NOT EXISTS idx_share_tickets_workspace_created
    ON maidan_share_tickets (workspace_id, created_at DESC);

CREATE TABLE IF NOT EXISTS maidan_share_ticket_artifacts (
    ticket_id TEXT NOT NULL REFERENCES maidan_share_tickets(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL REFERENCES maidan_artifacts(sha256) ON DELETE CASCADE,
    PRIMARY KEY (ticket_id, sha256)
);
