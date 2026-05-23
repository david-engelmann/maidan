-- Maidan API tokens, v0008, SQLite dialect.

CREATE TABLE maidan_api_tokens (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id     TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    token_hash    TEXT NOT NULL UNIQUE,
    label         TEXT,
    capabilities  TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    expires_at    TEXT,
    revoked_at    TEXT
);

CREATE INDEX idx_api_tokens_member ON maidan_api_tokens (member_id);
CREATE INDEX idx_api_tokens_workspace ON maidan_api_tokens (workspace_id);
