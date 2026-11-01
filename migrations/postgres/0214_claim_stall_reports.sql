-- The fencing tokens of the claims a ClaimStalled was last emitted for, one
-- per reason: no progress for the stall window, and a lease about to lapse.
-- One event per claim per reason, because a new claim mints a new token.
ALTER TABLE maidan_threads ADD COLUMN IF NOT EXISTS stall_reported_lease_id UUID;
ALTER TABLE maidan_threads ADD COLUMN IF NOT EXISTS lapse_warned_lease_id UUID;
