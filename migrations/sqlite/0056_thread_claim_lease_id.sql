-- Cluster 351 (mirror of postgres 0057): the claim fencing value (TEXT-encoded
-- UUID in SQLite). Minted when `assignee_id` is set, cleared on unassign.
ALTER TABLE maidan_threads ADD COLUMN claim_lease_id TEXT;
