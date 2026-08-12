-- Cluster 192: claim leases. A nullable lease deadline on an assigned thread.
-- When set and in the past, the assignment is reclaimable by the next claimer
-- (dead-agent recovery); NULL means a durable assignment with no lease. The
-- lease is a `claim_next` concept — manual assign / claim-specific stay durable.
ALTER TABLE maidan_threads ADD COLUMN assignment_expires_at TEXT;
