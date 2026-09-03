-- Cluster 351 (occupancy clocks): a fencing value for the current claim. A fresh
-- id is minted every time `assignee_id` is set (claim / claim_next / assign) and
-- cleared on unassign, so a claim-holder operation (renew, release) can prove it
-- still holds the lease the next owner hasn't already taken. A TTL lease alone is
-- not enough — the classic "the first holder unlocks the next owner's lock" bug.
-- Nullable: unassigned threads (and pre-existing rows) have no claim.
ALTER TABLE maidan_threads ADD COLUMN claim_lease_id UUID;
