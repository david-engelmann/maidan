-- Maidan federation peers + ingest dedupe, v0009, SQLite dialect.

CREATE TABLE maidan_peers (
    id                      TEXT PRIMARY KEY,
    workspace_id            TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name                    TEXT NOT NULL,
    base_url                TEXT NOT NULL,
    token_hash              TEXT NOT NULL UNIQUE,
    enabled                 INTEGER NOT NULL DEFAULT 1,
    last_synced_event_id    INTEGER NOT NULL DEFAULT 0,
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL
);

CREATE INDEX idx_peers_workspace ON maidan_peers (workspace_id);
CREATE INDEX idx_peers_enabled ON maidan_peers (enabled);

CREATE TABLE maidan_federated_ingest (
    peer_id           TEXT NOT NULL REFERENCES maidan_peers(id) ON DELETE CASCADE,
    remote_event_id   INTEGER NOT NULL,
    local_event_id    INTEGER NOT NULL,
    ingested_at       TEXT NOT NULL,
    PRIMARY KEY (peer_id, remote_event_id)
);

CREATE INDEX idx_federated_ingest_local ON maidan_federated_ingest (local_event_id);
