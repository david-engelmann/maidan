-- Second-family review routing (S4). A review child spawned when a change
-- enters review records the provider family of the change's author and the
-- family it is routed to; claim_next refuses it to any seat whose usage
-- evidence names the author's family. Set by the server only.
ALTER TABLE maidan_thread_attributes ADD COLUMN review_family TEXT;
ALTER TABLE maidan_thread_attributes ADD COLUMN author_family TEXT;
CREATE INDEX IF NOT EXISTS idx_usage_ledger_reporter ON maidan_usage_ledger (reporter_id);
