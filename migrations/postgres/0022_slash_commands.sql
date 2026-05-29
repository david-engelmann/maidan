-- Slash command registrations (Cluster 51.0).

CREATE TABLE maidan_slash_commands (
    id                 UUID PRIMARY KEY,
    workspace_id       UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    description        TEXT,
    handler_kind       TEXT NOT NULL,
    handler_target     TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL DEFAULT '',
    enabled            BOOLEAN NOT NULL DEFAULT TRUE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_slash_commands_name
    ON maidan_slash_commands (workspace_id, name)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_slash_commands_workspace
    ON maidan_slash_commands (workspace_id)
    WHERE revoked_at IS NULL;
