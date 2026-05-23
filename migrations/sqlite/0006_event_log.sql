-- Maidan persistent event log, v0006, SQLite dialect.

CREATE TABLE maidan_events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT NOT NULL,
    workspace_id  TEXT,
    channel_id    TEXT,
    thread_id     TEXT,
    payload       TEXT NOT NULL,
    occurred_at   TEXT NOT NULL
);

CREATE INDEX idx_events_workspace_id ON maidan_events (workspace_id, id);
