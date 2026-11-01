-- The claims a ClaimStalled was last emitted for, per reason. See the
-- Postgres twin.
ALTER TABLE maidan_threads ADD COLUMN stall_reported_lease_id TEXT;
ALTER TABLE maidan_threads ADD COLUMN lapse_warned_lease_id TEXT;
