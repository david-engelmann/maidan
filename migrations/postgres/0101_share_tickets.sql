-- Cluster 405: time-boxed, read-only capability tickets for sharing one
-- incident channel and an explicit artifact allowlist.
CREATE TABLE IF NOT EXISTS maidan_share_tickets (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    owner_id UUID NOT NULL REFERENCES maidan_members(id),
    created_by UUID NOT NULL REFERENCES maidan_members(id),
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);

CREATE INDEX IF NOT EXISTS idx_share_tickets_workspace_created
    ON maidan_share_tickets (workspace_id, created_at DESC);

CREATE TABLE IF NOT EXISTS maidan_share_ticket_artifacts (
    ticket_id UUID NOT NULL REFERENCES maidan_share_tickets(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL REFERENCES maidan_artifacts(sha256) ON DELETE CASCADE,
    PRIMARY KEY (ticket_id, sha256)
);
