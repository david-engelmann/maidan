-- Cluster 355 (W1): persisted steering guidance for a task/thread. The owner (or
-- a supervisor) records a durable instruction that survives claims and handoffs,
-- so a resuming or newly-assigned agent reads the CURRENT steer rather than
-- losing it with the previous claim. One steer per thread (upsert, latest wins).
-- Distinct from a Cluster-195 handoff note, which rides an assignment event and
-- is not persisted.
CREATE TABLE IF NOT EXISTS maidan_thread_steer (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    steer TEXT NOT NULL,
    steered_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    steered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
