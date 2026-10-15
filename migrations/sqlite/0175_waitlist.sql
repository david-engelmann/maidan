-- The hosted waitlist; see the Postgres twin (0175).
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
