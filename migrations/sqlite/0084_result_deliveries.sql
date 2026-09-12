-- Result-delivery state (Cluster 379.1; see postgres/0085). SQLite mirror:
-- timestamps are rfc3339 text bound by the store, so a revision comparison is a
-- string comparison — sound here because both sides of it are written by this
-- one module in the same format, never mixed with a `datetime('now')` column.
CREATE TABLE IF NOT EXISTS maidan_result_deliveries (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | failed | skipped
    external_ref TEXT,
    armed_revision TEXT NOT NULL,
    delivered_revision TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_result_deliveries_thread_target
    ON maidan_result_deliveries (thread_id, surface, selector);
