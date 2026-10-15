-- The hosted waitlist and the sign-up pending shape; see the Postgres
-- twin (0178).
--
-- One row per email address: `email_lower` carries the case-insensitive
-- uniqueness and matching, `email` keeps the address as the visitor typed
-- it. The invite token is never stored, only its SHA-256 hex; the token
-- itself is shown to the operator once, when the invite is minted.
-- `workspace_id` records what the sign-up created. No foreign key: the list
-- is the operator's record of who asked, and it outlives any workspace.
CREATE TABLE maidan_waitlist_entries (
    id                TEXT PRIMARY KEY,
    email             TEXT NOT NULL,
    email_lower       TEXT NOT NULL UNIQUE,
    use_case          TEXT NOT NULL,
    consented_at      TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'waiting',
    invite_token_hash TEXT,
    invite_expires_at TEXT,
    workspace_id      TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_waitlist_invite_hash ON maidan_waitlist_entries (invite_token_hash);
-- Sign-up pending rows.
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
