-- Outbound webhook subscriptions and delivery queue (Cluster 50.0).

CREATE TABLE maidan_webhook_subscriptions (
    id                 UUID PRIMARY KEY,
    workspace_id       UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    url                TEXT NOT NULL,
    label              TEXT,
    event_kinds        TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL,
    enabled            BOOLEAN NOT NULL DEFAULT TRUE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at         TIMESTAMPTZ
);

CREATE INDEX idx_webhook_subs_workspace
    ON maidan_webhook_subscriptions (workspace_id)
    WHERE revoked_at IS NULL;

CREATE TABLE maidan_webhook_deliveries (
    id               BIGSERIAL PRIMARY KEY,
    subscription_id  UUID NOT NULL REFERENCES maidan_webhook_subscriptions(id) ON DELETE CASCADE,
    log_id           BIGINT NOT NULL,
    payload          TEXT NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error       TEXT,
    delivered_at     TIMESTAMPTZ,
    quarantined_at   TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_webhook_deliveries_pending
    ON maidan_webhook_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;
