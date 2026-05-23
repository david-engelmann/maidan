-- Maidan persistent event log, v0006, Postgres dialect.

CREATE TABLE maidan_events (
    id            BIGSERIAL PRIMARY KEY,
    kind          TEXT NOT NULL,
    workspace_id  UUID,
    channel_id    UUID,
    thread_id     UUID,
    payload       JSONB NOT NULL,
    occurred_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_events_workspace_id ON maidan_events (workspace_id, id);
