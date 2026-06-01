-- Durable HTTP delivery for slash commands and FSM hooks (Cluster 68.0).

CREATE TABLE maidan_automation_deliveries (
    id               BIGSERIAL PRIMARY KEY,
    workspace_id     UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    source_kind      TEXT NOT NULL,
    source_id        UUID NOT NULL,
    target_url       TEXT NOT NULL,
    header_name      TEXT NOT NULL,
    header_value     TEXT NOT NULL,
    payload          TEXT NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error       TEXT,
    delivered_at     TIMESTAMPTZ,
    quarantined_at   TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_automation_deliveries_pending
    ON maidan_automation_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;

CREATE INDEX idx_automation_deliveries_workspace
    ON maidan_automation_deliveries (workspace_id, created_at DESC);
