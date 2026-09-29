-- When the current claim was taken, so the claim reaper can tell a holder
-- that never acknowledged it (ClaimUnacknowledged). Set with every new
-- fencing token and cleared with it.
ALTER TABLE maidan_threads ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ;
-- The fencing token of the claim a ClaimUnacknowledged was last emitted for:
-- one event per claim, because a new claim mints a new token.
ALTER TABLE maidan_threads ADD COLUMN IF NOT EXISTS unacknowledged_lease_id UUID;
-- Claims that are held but not yet acknowledged, the reaper's scan.
CREATE INDEX IF NOT EXISTS idx_threads_unacknowledged_claims
    ON maidan_threads (claimed_at)
    WHERE claim_lease_id IS NOT NULL AND work_started_at IS NULL;
