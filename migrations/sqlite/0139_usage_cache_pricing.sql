-- SQLite twin of postgres 0139.
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_write_5m_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_5m_tokens >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_1h_tokens >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_write_5m_price INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_5m_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_write_1h_price INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_1h_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN provider TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN service_tier TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN batch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_usage_ledger ADD COLUMN harness TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN harness_version TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_key TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN cache_miss_reason TEXT;
ALTER TABLE maidan_usage_ledger ADD COLUMN pack_sha256 TEXT;

UPDATE maidan_usage_ledger
SET cache_write_5m_tokens = cache_write_tokens,
    cache_write_5m_price = cache_write_price;

ALTER TABLE maidan_usage_ledger DROP COLUMN cache_write_tokens;
ALTER TABLE maidan_usage_ledger DROP COLUMN cache_write_price;

ALTER TABLE maidan_thread_budgets ADD COLUMN used_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_budgets ADD COLUMN used_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_budgets ADD COLUMN used_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_budgets ADD COLUMN used_cache_write_5m_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_budgets ADD COLUMN used_cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_usage_ledger_workspace_reporter
    ON maidan_usage_ledger (workspace_id, reporter_id);
