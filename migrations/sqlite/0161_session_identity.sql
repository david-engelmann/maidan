-- See the Postgres twin: an OIDC session keeps the identity that signed in.
ALTER TABLE maidan_sessions
    ADD COLUMN oidc_identity_id TEXT REFERENCES maidan_oidc_identities(id) ON DELETE CASCADE;
CREATE INDEX idx_sessions_oidc_identity ON maidan_sessions (oidc_identity_id)
    WHERE oidc_identity_id IS NOT NULL;
