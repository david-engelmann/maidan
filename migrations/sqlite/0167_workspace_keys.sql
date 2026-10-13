-- Per-tenant key custody; see the Postgres twin (0167).
CREATE TABLE maidan_workspace_keys (
    id              TEXT PRIMARY KEY NOT NULL,
    workspace_id    TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    custody         TEXT NOT NULL,
    wrapping_key_id TEXT,
    wrapped_key     BLOB,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    retired_at      TEXT,
    destroyed_at    TEXT,
    CHECK ((destroyed_at IS NULL) = (wrapped_key IS NOT NULL AND wrapping_key_id IS NOT NULL)),
    CHECK (destroyed_at IS NULL OR retired_at IS NOT NULL)
);
CREATE UNIQUE INDEX idx_workspace_keys_active ON maidan_workspace_keys (workspace_id)
    WHERE retired_at IS NULL;
CREATE INDEX idx_workspace_keys_wrapping ON maidan_workspace_keys (wrapping_key_id)
    WHERE destroyed_at IS NULL;

ALTER TABLE maidan_content_keys
    ADD COLUMN workspace_key_id TEXT REFERENCES maidan_workspace_keys(id);
CREATE INDEX idx_content_keys_workspace_key ON maidan_content_keys (workspace_key_id)
    WHERE shredded_at IS NULL;
