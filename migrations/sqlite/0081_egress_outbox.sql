-- Durable projector egress (Cluster 377.1; see postgres/0082). SQLite mirror:
-- timestamps are rfc3339 text bound by the store.
CREATE TABLE IF NOT EXISTS maidan_egress_outbox (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    source_log_id INTEGER NOT NULL,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | dead
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_egress_outbox_due
    ON maidan_egress_outbox (next_attempt_at)
    WHERE status = 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS idx_egress_outbox_dedup
    ON maidan_egress_outbox (source_log_id, surface, selector);
