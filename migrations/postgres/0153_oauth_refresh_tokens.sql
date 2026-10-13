-- OAuth 2.1 authorization server, phase three (Next 23). Refresh tokens.
-- Only the hash is stored, like the code hash. A refresh token is single
-- use: rotation sets used_at and issues a new one under the same grant and
-- lineage. Presenting a token whose used_at is set is a reuse, which revokes
-- every grant, access token and refresh token of the lineage.
--
-- lineage_id, client_id and resource are copied from the grant and the code
-- at issue, so a reuse is traced to its lineage from this row alone.

CREATE TABLE oauth_refresh_tokens (
    token_hash  TEXT PRIMARY KEY,
    grant_id    UUID NOT NULL REFERENCES oauth_grants(id) ON DELETE CASCADE,
    lineage_id  UUID NOT NULL,
    client_id   TEXT NOT NULL,
    -- RFC 8707: the resource the tokens are bound to. A refresh must name it.
    resource    TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    revoked_at  TIMESTAMPTZ
);

CREATE INDEX idx_oauth_refresh_lineage ON oauth_refresh_tokens (lineage_id);
CREATE INDEX idx_oauth_refresh_grant ON oauth_refresh_tokens (grant_id);

-- The lineage a code was issued under, written at consent. A used code that is
-- presented again revokes this lineage and no other: looking the grant up by
-- client, member, workspace and scope would find a newer grant the member
-- consented to after revoking the one this code belonged to. NULL only on
-- codes issued before this migration, which revoke nothing on reuse.
ALTER TABLE oauth_authorization_codes ADD COLUMN lineage_id UUID;
