-- OAuth 2.1 authorization server, phase three (Next 23). Refresh tokens.
-- SQLite dialect. See the Postgres file for the rationale.

CREATE TABLE oauth_refresh_tokens (
    token_hash  TEXT PRIMARY KEY,
    grant_id    TEXT NOT NULL REFERENCES oauth_grants(id) ON DELETE CASCADE,
    lineage_id  TEXT NOT NULL,
    client_id   TEXT NOT NULL,
    resource    TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    used_at     TEXT,
    revoked_at  TEXT
);

CREATE INDEX idx_oauth_refresh_lineage ON oauth_refresh_tokens (lineage_id);
CREATE INDEX idx_oauth_refresh_grant ON oauth_refresh_tokens (grant_id);
