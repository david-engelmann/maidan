-- A browser session can be made from an API token (`POST
-- /auth/session/from-token`). It keeps the token's id, and each request on it
-- re-resolves that token, so the session has the token's authority and ends
-- when the token is revoked, rotated or expires. An OIDC session has none.
-- Deleting the token deletes its sessions.
ALTER TABLE maidan_sessions
    ADD COLUMN api_token_id UUID REFERENCES maidan_api_tokens(id) ON DELETE CASCADE;
CREATE INDEX idx_sessions_api_token ON maidan_sessions (api_token_id)
    WHERE api_token_id IS NOT NULL;

-- Never read: SameSite=Lax, JSON-only writes and the Origin check on unsafe
-- session requests are the CSRF defence.
ALTER TABLE maidan_sessions DROP COLUMN csrf_secret;
