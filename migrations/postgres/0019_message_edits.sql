-- Per-edit body history for messages (Cluster 46.0).

CREATE TABLE maidan_message_edits (
    id          BIGSERIAL PRIMARY KEY,
    message_id  UUID NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    editor_id   UUID NOT NULL REFERENCES maidan_members(id),
    body_before TEXT NOT NULL,
    body_after  TEXT NOT NULL,
    edited_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_message_edits_message ON maidan_message_edits (message_id, edited_at);
