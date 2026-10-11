-- OAuth 2.1 authorization server, phase two (Next 23). Client registry,
-- authorization codes, and grants. Tokens issued through OAuth live in
-- maidan_api_tokens with oauth_grant_id set; they never carry
-- approval:grant (enforced at mint time, not by the schema).
--
-- No dynamic client registration: clients are pre-registered here or
-- identified by client metadata documents (P1).

CREATE TABLE oauth_clients (
    id                 UUID PRIMARY KEY,
    client_id          TEXT NOT NULL UNIQUE,
    name               TEXT NOT NULL,
    -- JSON array of exact redirect URIs. Match is exact, no prefix games.
    redirect_uris      TEXT NOT NULL,
    -- NULL for public clients (PKCE only). Hashed for confidential clients.
    client_secret_hash TEXT,
    -- JSON array of allowed scopes (capability names).
    allowed_scopes     TEXT NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at         TIMESTAMPTZ
);

CREATE TABLE oauth_authorization_codes (
    -- Hash of the code, like token_hash. The code itself is shown once.
    code_hash            TEXT PRIMARY KEY,
    client_id            TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    member_id            UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id         UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    redirect_uri         TEXT NOT NULL,
    -- PKCE: S256 only. The challenge as sent; the verifier is checked at
    -- the token endpoint.
    code_challenge        TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL CHECK (code_challenge_method = 'S256'),
    -- JSON array of granted scopes (capability names, subset of allowed).
    scope                TEXT NOT NULL,
    -- RFC 8707 resource indicator, the MCP endpoint this grant is for.
    resource             TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at           TIMESTAMPTZ NOT NULL,
    -- Single use: set when exchanged. A second exchange with the same code
    -- is refused, and (P3) revokes the grant lineage.
    used_at              TIMESTAMPTZ
);

CREATE INDEX idx_oauth_codes_client ON oauth_authorization_codes (client_id);
CREATE INDEX idx_oauth_codes_expires ON oauth_authorization_codes (expires_at);

CREATE TABLE oauth_grants (
    id           UUID PRIMARY KEY,
    client_id    TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    member_id    UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    -- JSON array of granted scopes. Never includes approval:grant.
    scope        TEXT NOT NULL,
    -- All tokens from one grant share this. A reused refresh token (P3)
    -- revokes everything under the lineage id.
    lineage_id   UUID NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at   TIMESTAMPTZ
);

CREATE INDEX idx_oauth_grants_lineage ON oauth_grants (lineage_id);
CREATE INDEX idx_oauth_grants_member ON oauth_grants (member_id);

-- Which grant an OAuth-issued token came from. NULL for all other tokens.
ALTER TABLE maidan_api_tokens ADD COLUMN oauth_grant_id UUID REFERENCES oauth_grants(id) ON DELETE CASCADE;
CREATE INDEX idx_api_tokens_oauth_grant ON maidan_api_tokens (oauth_grant_id);
