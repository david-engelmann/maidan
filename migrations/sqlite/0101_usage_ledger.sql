-- Cluster 410 (Postgres 0102 twin): idempotent model-usage ledger.
CREATE TABLE IF NOT EXISTS maidan_usage_ledger (
    usage_report_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reporter_id TEXT NOT NULL REFERENCES maidan_members(id),
    claim_lease_id TEXT NOT NULL,
    model TEXT NOT NULL CHECK (length(trim(model)) BETWEEN 1 AND 255),
    input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
    output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
    cache_read_tokens INTEGER NOT NULL CHECK (cache_read_tokens >= 0),
    cache_write_tokens INTEGER NOT NULL CHECK (cache_write_tokens >= 0),
    input_price INTEGER NOT NULL CHECK (input_price >= 0),
    output_price INTEGER NOT NULL CHECK (output_price >= 0),
    cache_read_price INTEGER NOT NULL CHECK (cache_read_price >= 0),
    cache_write_price INTEGER NOT NULL CHECK (cache_write_price >= 0),
    usd_micros INTEGER NOT NULL CHECK (usd_micros >= 0),
    turns INTEGER NOT NULL CHECK (turns >= 0),
    budget TEXT,
    stopped INTEGER,
    reason TEXT,
    -- No FK: event retention must not be pinned by the economic ledger.
    usage_event_id INTEGER,
    claim_failed_event_id INTEGER,
    accepted_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK ((budget IS NULL) = (stopped IS NULL)),
    CHECK ((budget IS NULL) = (usage_event_id IS NULL)),
    CHECK (
        budget IS NULL OR
        (stopped = 0 AND reason IS NULL AND claim_failed_event_id IS NULL) OR
        (stopped = 1 AND reason IS NOT NULL AND claim_failed_event_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_usage_ledger_thread_accepted
    ON maidan_usage_ledger (thread_id, accepted_at DESC);
