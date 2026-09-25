-- Withdrawn messages a legal hold keeps; see the Postgres twin (0111).
CREATE TABLE maidan_preserved_messages (
    message_id    TEXT PRIMARY KEY NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    body          TEXT NOT NULL,
    content       TEXT,
    tombstoned_at TEXT NOT NULL
);
CREATE INDEX idx_preserved_messages_workspace
    ON maidan_preserved_messages (workspace_id, tombstoned_at);
