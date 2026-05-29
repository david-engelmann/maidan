-- Outbound webhook subscriptions and delivery queue (Cluster 50.0).

CREATE TABLE maidan_webhook_subscriptions (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    url                TEXT NOT NULL,
    label              TEXT,
    event_kinds        TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL,
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at         TEXT
);

CREATE INDEX idx_webhook_subs_workspace
    ON maidan_webhook_subscriptions (workspace_id);

CREATE TABLE maidan_webhook_deliveries (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    subscription_id  TEXT NOT NULL REFERENCES maidan_webhook_subscriptions(id) ON DELETE CASCADE,
    log_id           INTEGER NOT NULL,
    payload          TEXT NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_error       TEXT,
    delivered_at     TEXT,
    quarantined_at   TEXT,
    created_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_webhook_deliveries_pending
    ON maidan_webhook_deliveries (next_attempt_at);
