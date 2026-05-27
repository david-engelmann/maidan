-- Outbox relay quarantine (Cluster 12.0).

ALTER TABLE maidan_outbox
    ADD COLUMN IF NOT EXISTS quarantined_at TIMESTAMPTZ;

DROP INDEX IF EXISTS idx_outbox_pending;

CREATE INDEX idx_outbox_relayable ON maidan_outbox (id)
    WHERE published_at IS NULL AND quarantined_at IS NULL;
