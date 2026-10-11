-- OAuth 2.1 authorization server, consent step (Next 23). SQLite dialect.
-- No foreign keys: ephemeral table, application validates references.

CREATE TABLE oauth_pending_requests (
    id               TEXT PRIMARY KEY,
    client_id        TEXT NOT NULL,
    member_id        TEXT NOT NULL,
    workspace_id     TEXT NOT NULL,
    redirect_uri     TEXT NOT NULL,
    code_challenge   TEXT NOT NULL,
    scope            TEXT NOT NULL,
    resource         TEXT,
    state            TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    expires_at       TEXT NOT NULL
);

CREATE INDEX idx_oauth_pending_member ON oauth_pending_requests (member_id);
CREATE INDEX idx_oauth_pending_expires ON oauth_pending_requests (expires_at);
