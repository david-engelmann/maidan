-- What a legal hold keeps of a message withdrawn while it holds. A tombstone
-- blanks the live row for everyone; under a hold its words are copied here
-- first, so every existing reader still sees nothing, and only the audited
-- preserved-content read (token:admin) sees them. Lifting the hold deletes
-- them. Earlier versions stay in maidan_message_edits for the same span.
CREATE TABLE maidan_preserved_messages (
    message_id    UUID PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
    workspace_id  UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    body          TEXT NOT NULL,
    content       JSONB,
    tombstoned_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_preserved_messages_workspace
    ON maidan_preserved_messages (workspace_id, tombstoned_at);
