-- When the current claim was taken, and the claim a ClaimUnacknowledged was
-- last emitted for. See the Postgres twin.
ALTER TABLE maidan_threads ADD COLUMN claimed_at TEXT;
ALTER TABLE maidan_threads ADD COLUMN unacknowledged_lease_id TEXT;
CREATE INDEX IF NOT EXISTS idx_threads_unacknowledged_claims
    ON maidan_threads (claimed_at)
    WHERE claim_lease_id IS NOT NULL AND work_started_at IS NULL;
