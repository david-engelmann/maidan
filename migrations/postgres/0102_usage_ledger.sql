-- Cluster 410: idempotent, claim-fenced model-usage ledger. Outcome columns
-- preserve the exact response returned to an economic retry.
CREATE TABLE IF NOT EXISTS maidan_usage_ledger (
    usage_report_id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reporter_id UUID NOT NULL REFERENCES maidan_members(id),
    claim_lease_id UUID NOT NULL,
    model TEXT NOT NULL CHECK (length(trim(model)) BETWEEN 1 AND 255),
    input_tokens BIGINT NOT NULL CHECK (input_tokens >= 0),
    output_tokens BIGINT NOT NULL CHECK (output_tokens >= 0),
    cache_read_tokens BIGINT NOT NULL CHECK (cache_read_tokens >= 0),
    cache_write_tokens BIGINT NOT NULL CHECK (cache_write_tokens >= 0),
    input_price BIGINT NOT NULL CHECK (input_price >= 0),
    output_price BIGINT NOT NULL CHECK (output_price >= 0),
    cache_read_price BIGINT NOT NULL CHECK (cache_read_price >= 0),
    cache_write_price BIGINT NOT NULL CHECK (cache_write_price >= 0),
    usd_micros BIGINT NOT NULL CHECK (usd_micros >= 0),
    turns BIGINT NOT NULL CHECK (turns >= 0),
    budget JSONB,
    stopped BOOLEAN,
    reason TEXT,
    -- Event ids are durable pointers, not FKs: retention may prune the event
    -- row while the economic ledger must remain queryable.
    usage_event_id BIGINT,
    claim_failed_event_id BIGINT,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((budget IS NULL) = (stopped IS NULL)),
    CHECK ((budget IS NULL) = (usage_event_id IS NULL)),
    CHECK (
        budget IS NULL OR
        (stopped = FALSE AND reason IS NULL AND claim_failed_event_id IS NULL) OR
        (stopped = TRUE AND reason IS NOT NULL AND claim_failed_event_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_usage_ledger_thread_accepted
    ON maidan_usage_ledger (thread_id, accepted_at DESC);
