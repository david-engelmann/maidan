-- Maidan transactional outbox for SQLite relay (Cluster 14.0).

CREATE TABLE maidan_outbox (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    log_id         INTEGER NOT NULL REFERENCES maidan_events (id) ON DELETE CASCADE,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    published_at   TEXT,
    attempts       INTEGER NOT NULL DEFAULT 0,
    quarantined_at TEXT
);

CREATE INDEX idx_outbox_relayable ON maidan_outbox (id)
    WHERE published_at IS NULL AND quarantined_at IS NULL;
