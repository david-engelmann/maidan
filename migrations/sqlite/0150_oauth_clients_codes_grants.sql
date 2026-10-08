-- OAuth 2.1 authorization server, phase two (Next 23). Client registry,
-- authorization codes, and grants. SQLite dialect.
--
-- No dynamic client registration: clients are pre-registered here or
-- identified by client metadata documents (P1).

CREATE TABLE oauth_clients (
    id                 TEXT PRIMARY KEY,
    client_id          TEXT NOT NULL UNIQUE,
    name               TEXT NOT NULL,
    redirect_uris      TEXT NOT NULL,
    client_secret_hash TEXT,
    allowed_scopes     TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    revoked_at         TEXT
);

CREATE TABLE oauth_authorization_codes (
    code_hash            TEXT PRIMARY KEY,
    client_id            TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    member_id            TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id         TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    redirect_uri         TEXT NOT NULL,
    code_challenge        TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL CHECK (code_challenge_method = 'S256'),
    scope                TEXT NOT NULL,
    resource             TEXT,
    created_at           TEXT NOT NULL,
    expires_at           TEXT NOT NULL,
    used_at              TEXT
);

CREATE INDEX idx_oauth_codes_client ON oauth_authorization_codes (client_id);
CREATE INDEX idx_oauth_codes_expires ON oauth_authorization_codes (expires_at);

CREATE TABLE oauth_grants (
    id           TEXT PRIMARY KEY,
    client_id    TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    member_id    TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    scope        TEXT NOT NULL,
    lineage_id   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    revoked_at   TEXT
);

CREATE INDEX idx_oauth_grants_lineage ON oauth_grants (lineage_id);
CREATE INDEX idx_oauth_grants_member ON oauth_grants (member_id);

ALTER TABLE maidan_api_tokens ADD COLUMN oauth_grant_id TEXT REFERENCES oauth_grants(id) ON DELETE CASCADE;
CREATE INDEX idx_api_tokens_oauth_grant ON maidan_api_tokens (oauth_grant_id);
