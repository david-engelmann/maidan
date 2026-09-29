-- The claim reaper looks for leases that lapsed every few seconds. See the
-- Postgres twin.
CREATE INDEX IF NOT EXISTS idx_threads_lease_deadline
    ON maidan_threads (assignment_expires_at)
    WHERE assignment_expires_at IS NOT NULL;
