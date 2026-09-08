-- Cluster 355 (W1, mirror of postgres 0059): a durable OWNER axis on threads,
-- distinct from the assignee (the claimer). ON DELETE SET NULL mirrors
-- assignee_id (Cluster 171).
ALTER TABLE maidan_threads
    ADD COLUMN owner_id TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;

CREATE INDEX idx_threads_owner ON maidan_threads (owner_id);
