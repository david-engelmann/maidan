-- Slash command registrations (Cluster 51.0).

CREATE TABLE maidan_slash_commands (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    description        TEXT,
    handler_kind       TEXT NOT NULL,
    handler_target     TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL DEFAULT '',
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at         TEXT
);

CREATE UNIQUE INDEX idx_slash_commands_name
    ON maidan_slash_commands (workspace_id, name)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_slash_commands_workspace
    ON maidan_slash_commands (workspace_id);
