-- Per-edit body history for messages (Cluster 46.0).

CREATE TABLE maidan_message_edits (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id  TEXT NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    editor_id   TEXT NOT NULL REFERENCES maidan_members(id),
    body_before TEXT NOT NULL,
    body_after  TEXT NOT NULL,
    edited_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_message_edits_message ON maidan_message_edits (message_id, edited_at);
