-- See the Postgres twin: a pending sign-in has a kind, `front_door` names no
-- workspace, and the workspace id has no foreign key, so login need not look
-- the workspace up before redirecting.
--
-- SQLite cannot drop a NOT NULL or a foreign key, so the table is rebuilt. No
-- table references it.
CREATE TABLE maidan_oidc_pending_rebuilt (
    state          TEXT PRIMARY KEY,
    kind           TEXT NOT NULL DEFAULT 'sign_in',
    workspace_id   TEXT,
    nonce          TEXT NOT NULL,
    pkce_verifier  TEXT NOT NULL,
    return_to      TEXT,
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL)
        OR (kind = 'front_door' AND workspace_id IS NULL)
    )
);
INSERT INTO maidan_oidc_pending_rebuilt
    (state, kind, workspace_id, nonce, pkce_verifier, return_to, created_at, expires_at)
SELECT state, 'sign_in', workspace_id, nonce, pkce_verifier, return_to, created_at, expires_at
FROM maidan_oidc_pending;
DROP TABLE maidan_oidc_pending;
ALTER TABLE maidan_oidc_pending_rebuilt RENAME TO maidan_oidc_pending;

-- The front door finds an identity's workspaces by issuer and subject alone.
CREATE INDEX idx_oidc_identities_subject ON maidan_oidc_identities (issuer, subject);
