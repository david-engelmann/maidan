-- Cluster 363 (G3): park a thread from dispatch. A row marks the thread
-- "unclaimable" with a reason — `claim_next` skips it and an explicit `claim` is
-- refused — until the row is cleared. Distinct from blocked-by-deps, blocked-by-gate,
-- and skill-miss: this is an explicit human/owner park (needs triage, waiting on
-- external, broken). Presence = unclaimable; absence = claimable.
CREATE TABLE maidan_thread_unclaimable (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    marked_by UUID NOT NULL REFERENCES maidan_members(id),
    marked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
