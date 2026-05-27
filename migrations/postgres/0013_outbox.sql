-- Maidan transactional outbox for Postgres bus relay (Cluster 10.0).

CREATE TABLE maidan_outbox (
    id           BIGSERIAL PRIMARY KEY,
    log_id       BIGINT NOT NULL REFERENCES maidan_events (id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ,
    attempts     INT NOT NULL DEFAULT 0
);

CREATE INDEX idx_outbox_pending ON maidan_outbox (id)
    WHERE published_at IS NULL;
