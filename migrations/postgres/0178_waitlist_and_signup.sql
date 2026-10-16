-- The hosted waitlist and the sign-up pending shape (Next 26).
--
-- One row per email address: `email_lower` carries the case-insensitive
-- uniqueness and matching, `email` keeps the address as the visitor typed
-- it. The invite token is never stored, only its SHA-256 hex; the token
-- itself is shown to the operator once, when the invite is minted.
-- `workspace_id` records what the sign-up created. No foreign key: the list
-- is the operator's record of who asked, and it outlives any workspace.
CREATE TABLE maidan_waitlist_entries (
    id                UUID PRIMARY KEY,
    email             TEXT NOT NULL,
    email_lower       TEXT NOT NULL UNIQUE,
    use_case          TEXT NOT NULL,
    consented_at      TIMESTAMPTZ NOT NULL,
    status            TEXT NOT NULL DEFAULT 'waiting',
    invite_token_hash TEXT,
    invite_expires_at TIMESTAMPTZ,
    workspace_id      UUID,
    created_at        TIMESTAMPTZ NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_waitlist_invite_hash ON maidan_waitlist_entries (invite_token_hash);
-- Sign-up pending rows.
--
-- A pending sign-up (`kind = 'sign_up'`) names no workspace — the callback
-- creates one — and may carry the waitlist entry whose invite gates it.
-- The kind CHECK grows the third shape beside 0163's two, so a sign-up
-- `state` can never complete a sign-in or the other way round.
ALTER TABLE maidan_oidc_pending
    ADD COLUMN invite_entry_id UUID;
ALTER TABLE maidan_oidc_pending
    DROP CONSTRAINT maidan_oidc_pending_kind_workspace;
ALTER TABLE maidan_oidc_pending
    ADD CONSTRAINT maidan_oidc_pending_kind_workspace CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL)
        OR (kind = 'front_door' AND workspace_id IS NULL)
        OR (kind = 'sign_up' AND workspace_id IS NULL)
    );
