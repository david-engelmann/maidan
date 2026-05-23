-- Maidan API tokens, v0008, Postgres dialect.

CREATE TABLE maidan_api_tokens (
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id     UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    token_hash    TEXT NOT NULL UNIQUE,
    label         TEXT,
    capabilities  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);

CREATE INDEX idx_api_tokens_member ON maidan_api_tokens (member_id);
CREATE INDEX idx_api_tokens_workspace ON maidan_api_tokens (workspace_id);
