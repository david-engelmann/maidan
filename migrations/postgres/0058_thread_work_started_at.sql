-- Cluster 351 (occupancy clocks): the second clock. `assignment_expires_at` is
-- the *claim* clock (when the lease lapses); this is the *working* clock — when
-- the claim-holder actually acknowledged and began work. NULL = claimed but not
-- yet started (or unassigned). Reset to NULL on every (re)claim/assign/unassign
-- so it always measures the CURRENT claim epoch, then stamped by
-- `acknowledge_claim`. Splitting the two clocks is what lets occupancy tell a
-- stalled-before-start agent from a stalled-mid-work one.
ALTER TABLE maidan_threads ADD COLUMN work_started_at TIMESTAMPTZ;
