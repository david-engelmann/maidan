-- Cluster 365 (G3 fair dispatch): per-thread dispatch priority. Absence = the
-- default priority 0 (normal); higher = more urgent. `claim_next` orders by an
-- effective rank that ages a thread's base priority up the longer it has waited,
-- so a high-priority task jumps the queue but a long-waiting normal task is never
-- starved. A side table (not a threads column) keeps the ~46-site row_to_thread
-- ripple off the hot path.
CREATE TABLE IF NOT EXISTS maidan_thread_priorities (
    thread_id  UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    priority   BIGINT NOT NULL DEFAULT 0,
    set_by     UUID NOT NULL REFERENCES maidan_members(id),
    set_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
