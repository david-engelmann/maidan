-- Sign-up pending rows; see the Postgres twin (0176).
--
-- A pending sign-up (`kind = 'sign_up'`) names no workspace — the callback
-- creates one — and may carry the waitlist entry whose invite gates it.
-- Like 0163, the CHECK ties each kind to its shape, so a sign-up `state`
-- can never complete a sign-in or the other way round.
--
-- SQLite cannot drop or replace a CHECK, so the table is rebuilt. No table
-- references it.
CREATE TABLE maidan_oidc_pending_rebuilt (
    state           TEXT PRIMARY KEY,
    kind            TEXT NOT NULL DEFAULT 'sign_in',
    workspace_id    TEXT,
    nonce           TEXT NOT NULL,
    pkce_verifier   TEXT NOT NULL,
    return_to       TEXT,
    invite_entry_id TEXT,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL)
        OR (kind = 'front_door' AND workspace_id IS NULL)
        OR (kind = 'sign_up' AND workspace_id IS NULL)
    )
);
INSERT INTO maidan_oidc_pending_rebuilt
    (state, kind, workspace_id, nonce, pkce_verifier, return_to, invite_entry_id, created_at, expires_at)
SELECT state, kind, workspace_id, nonce, pkce_verifier, return_to, NULL, created_at, expires_at
FROM maidan_oidc_pending;
DROP TABLE maidan_oidc_pending;
ALTER TABLE maidan_oidc_pending_rebuilt RENAME TO maidan_oidc_pending;
