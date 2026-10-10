-- An OIDC session records the identity (issuer and subject, as the
-- `maidan_oidc_identities` row of the workspace it signed in to) that signed
-- in, so the workspace switcher lists exactly that identity's workspaces and
-- never derives them from the member, which can hold more than one linked
-- subject. A session made from a token, and one from before this migration,
-- has none. Deleting the identity row deletes its sessions.
ALTER TABLE maidan_sessions
    ADD COLUMN oidc_identity_id UUID REFERENCES maidan_oidc_identities(id) ON DELETE CASCADE;
CREATE INDEX idx_sessions_oidc_identity ON maidan_sessions (oidc_identity_id)
    WHERE oidc_identity_id IS NOT NULL;
