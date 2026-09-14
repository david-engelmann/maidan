-- Cluster 386 (Wave 2 #27, G14 + W2): explicit blocked-reason. A row parks
-- the thread from `claim_next` with a closed reason
-- (`dag|gate|human|child|quota|unclaimable`). Distinct from Cluster 217/218
-- DAG readiness (deps-must-be-terminal is derived, not stored here) and from
-- Cluster 363's free-text park table. Presence = blocked; absence = unblocked.
-- Numbered 0089: Cluster 385 LandGate took postgres 0088.
CREATE TABLE maidan_thread_blocks (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN ('dag', 'gate', 'human', 'child', 'quota', 'unclaimable')),
    set_by UUID NOT NULL REFERENCES maidan_members(id),
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
