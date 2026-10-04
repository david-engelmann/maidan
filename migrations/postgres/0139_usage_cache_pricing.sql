-- Cache-priced usage. input_tokens stays uncached input. Cache writes split
-- into a 5-minute tier and a 1-hour tier. Existing unsplit writes land on the
-- 5-minute tier, which was the only tier the previous column could mean.
-- Evidence columns are null when the reporter did not send them.

ALTER TABLE maidan_usage_ledger
    ADD COLUMN cache_write_5m_tokens BIGINT NOT NULL DEFAULT 0 CHECK (cache_write_5m_tokens >= 0),
    ADD COLUMN cache_write_1h_tokens BIGINT NOT NULL DEFAULT 0 CHECK (cache_write_1h_tokens >= 0),
    ADD COLUMN cache_write_5m_price BIGINT NOT NULL DEFAULT 0 CHECK (cache_write_5m_price >= 0),
    ADD COLUMN cache_write_1h_price BIGINT NOT NULL DEFAULT 0 CHECK (cache_write_1h_price >= 0),
    ADD COLUMN provider TEXT,
    ADD COLUMN service_tier TEXT,
    ADD COLUMN batch BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN harness TEXT,
    ADD COLUMN harness_version TEXT,
    ADD COLUMN cache_key TEXT,
    ADD COLUMN cache_miss_reason TEXT,
    ADD COLUMN pack_sha256 TEXT;

UPDATE maidan_usage_ledger
SET cache_write_5m_tokens = cache_write_tokens,
    cache_write_5m_price = cache_write_price;

ALTER TABLE maidan_usage_ledger DROP COLUMN cache_write_tokens;
ALTER TABLE maidan_usage_ledger DROP COLUMN cache_write_price;

ALTER TABLE maidan_thread_budgets
    ADD COLUMN used_input_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN used_output_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN used_cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN used_cache_write_5m_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN used_cache_write_1h_tokens BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_usage_ledger_workspace_reporter
    ON maidan_usage_ledger (workspace_id, reporter_id);
