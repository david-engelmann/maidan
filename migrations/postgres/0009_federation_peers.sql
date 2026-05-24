-- Maidan federation peers + ingest dedupe, v0009, Postgres dialect.

CREATE TABLE maidan_peers (
    id                      UUID PRIMARY KEY,
    workspace_id            UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name                    TEXT NOT NULL,
    base_url                TEXT NOT NULL,
    token_hash              TEXT NOT NULL UNIQUE,
    enabled                 BOOLEAN NOT NULL DEFAULT TRUE,
    last_synced_event_id    BIGINT NOT NULL DEFAULT 0,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_peers_workspace ON maidan_peers (workspace_id);
CREATE INDEX idx_peers_enabled ON maidan_peers (enabled) WHERE enabled = TRUE;

CREATE TABLE maidan_federated_ingest (
    peer_id           UUID NOT NULL REFERENCES maidan_peers(id) ON DELETE CASCADE,
    remote_event_id   BIGINT NOT NULL,
    local_event_id    BIGINT NOT NULL REFERENCES maidan_events(id) ON DELETE CASCADE,
    ingested_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (peer_id, remote_event_id)
);

CREATE INDEX idx_federated_ingest_local ON maidan_federated_ingest (local_event_id);
