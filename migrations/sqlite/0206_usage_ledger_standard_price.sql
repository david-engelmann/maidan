-- The standard-tier price for a call in a cheaper lane (Context Economics
-- C7c); see the Postgres twin. SQLite cannot add a table constraint, so the
-- all-or-none rule is kept by the store, which writes the five together.
ALTER TABLE maidan_usage_ledger ADD COLUMN standard_input_price INTEGER CHECK (standard_input_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN standard_output_price INTEGER CHECK (standard_output_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN standard_cache_read_price INTEGER CHECK (standard_cache_read_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN standard_cache_write_5m_price INTEGER CHECK (standard_cache_write_5m_price >= 0);
ALTER TABLE maidan_usage_ledger ADD COLUMN standard_cache_write_1h_price INTEGER CHECK (standard_cache_write_1h_price >= 0);
