-- Crypto-shredding: one wrapped data key per message; see the Postgres twin (0113).
CREATE TABLE maidan_content_keys (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    kek_id       TEXT,
    wrapped_key  BLOB,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    shredded_at  TEXT,
    CHECK ((shredded_at IS NULL) = (wrapped_key IS NOT NULL AND kek_id IS NOT NULL))
);
CREATE INDEX idx_content_keys_workspace ON maidan_content_keys (workspace_id);
CREATE INDEX idx_content_keys_kek ON maidan_content_keys (kek_id)
    WHERE shredded_at IS NULL;

ALTER TABLE maidan_events
    ADD COLUMN content_key_id TEXT REFERENCES maidan_content_keys(id);
CREATE INDEX idx_events_content_key ON maidan_events (content_key_id)
    WHERE content_key_id IS NOT NULL;
