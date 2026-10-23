-- Second-family review routing (S4); see the Postgres twin.
ALTER TABLE maidan_thread_attributes ADD COLUMN review_family TEXT;
ALTER TABLE maidan_thread_attributes ADD COLUMN author_family TEXT;
CREATE INDEX IF NOT EXISTS idx_usage_ledger_reporter ON maidan_usage_ledger (reporter_id);
