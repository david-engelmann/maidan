-- OAuth 2.1 authorization server, consent step (Next 23). A pending request
-- holds the validated authorize parameters while the member decides on the
-- consent page. The consent POST consumes it (single use); it expires in ten
-- minutes. The member_id is bound from the session, so one member cannot
-- approve another's request.
--
-- No foreign keys: the table is ephemeral (10-minute TTL, single use) and
-- the application validates client, member and workspace on every read.

CREATE TABLE oauth_pending_requests (
    id               UUID PRIMARY KEY,
    client_id        TEXT NOT NULL,
    member_id        UUID NOT NULL,
    workspace_id     UUID NOT NULL,
    redirect_uri     TEXT NOT NULL,
    code_challenge   TEXT NOT NULL,
    -- JSON array of granted scopes (already validated as delegatable).
    scope            TEXT NOT NULL,
    resource         TEXT,
    -- Opaque state the client sent; echoed back on the redirect.
    state            TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_oauth_pending_member ON oauth_pending_requests (member_id);
CREATE INDEX idx_oauth_pending_expires ON oauth_pending_requests (expires_at);
