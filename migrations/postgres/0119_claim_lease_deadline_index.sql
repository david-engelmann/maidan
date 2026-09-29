-- The claim reaper looks for leases that lapsed every few seconds. Only a
-- leased claim has a deadline, so a partial index keeps the scan to the
-- claims that can lapse instead of every thread.
CREATE INDEX IF NOT EXISTS idx_threads_lease_deadline
    ON maidan_threads (assignment_expires_at)
    WHERE assignment_expires_at IS NOT NULL;
