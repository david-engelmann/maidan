-- Cluster 364 (G2/G4): wait-edges + on_timeout escalation. A row is a durable
-- timer on a thread: it is waiting until `wait_until`, and on timeout the
-- `on_timeout` escalation policy fires — never a decision (no auto close/approve/
-- decline); the room reaches a human (notify) and optionally parks the thread.
-- Either cancelled (satisfied — the awaited thing happened) or fired by the
-- sweeper (`fired_at` set). One wait per thread.
CREATE TABLE maidan_thread_waits (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    wait_until TIMESTAMPTZ NOT NULL,
    on_timeout TEXT NOT NULL,
    reason TEXT,
    created_by UUID NOT NULL REFERENCES maidan_members(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    fired_at TIMESTAMPTZ
);

-- The sweeper scans un-fired waits by deadline.
CREATE INDEX idx_thread_waits_due ON maidan_thread_waits (wait_until) WHERE fired_at IS NULL;
