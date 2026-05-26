-- OIDC identities, browser sessions, and pending auth state, v0012, SQLite.

CREATE TABLE maidan_oidc_identities (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    issuer        TEXT NOT NULL,
    subject       TEXT NOT NULL,
    member_id     TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    email         TEXT,
    created_at    TEXT NOT NULL,
    last_login_at TEXT NOT NULL,
    UNIQUE (workspace_id, issuer, subject)
);

CREATE INDEX idx_oidc_identities_member ON maidan_oidc_identities (member_id);

CREATE TABLE maidan_sessions (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id     TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    csrf_secret   TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    expires_at    TEXT NOT NULL
);

CREATE INDEX idx_sessions_expires ON maidan_sessions (expires_at);

CREATE TABLE maidan_oidc_pending (
    state          TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    nonce          TEXT NOT NULL,
    pkce_verifier  TEXT NOT NULL,
    return_to      TEXT,
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL
);
