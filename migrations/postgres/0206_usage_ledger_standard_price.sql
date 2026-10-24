-- The standard-tier price a reporter gives for a call in a cheaper lane
-- (batch, or a batch or flex service tier), so the ledger can show the lane
-- saving (Context Economics C7c). All five are set together or all are NULL.
-- No backfill: older rows carry no standard price and count as unpriced.
ALTER TABLE maidan_usage_ledger
    ADD COLUMN standard_input_price BIGINT CHECK (standard_input_price >= 0),
    ADD COLUMN standard_output_price BIGINT CHECK (standard_output_price >= 0),
    ADD COLUMN standard_cache_read_price BIGINT CHECK (standard_cache_read_price >= 0),
    ADD COLUMN standard_cache_write_5m_price BIGINT CHECK (standard_cache_write_5m_price >= 0),
    ADD COLUMN standard_cache_write_1h_price BIGINT CHECK (standard_cache_write_1h_price >= 0),
    ADD CONSTRAINT maidan_usage_ledger_standard_price_whole CHECK (
        (standard_input_price IS NULL) = (standard_output_price IS NULL)
        AND (standard_input_price IS NULL) = (standard_cache_read_price IS NULL)
        AND (standard_input_price IS NULL) = (standard_cache_write_5m_price IS NULL)
        AND (standard_input_price IS NULL) = (standard_cache_write_1h_price IS NULL)
    );
