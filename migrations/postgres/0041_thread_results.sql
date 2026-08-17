-- Cluster 234 (Program B, Arc F): a task's structured result. When an agent
-- finishes a task (thread) it can attach one structured `result` (arbitrary
-- JSON); a requester — or a parent task that depends on it — reads that result
-- back. One result per thread (upsert on re-set). Coordination waits (later
-- clusters) block on a `ThreadResultSet` event. No routes/worker in this cluster
-- — the zero-blast-radius foundation pattern (Cluster 159 / 217 / 226 / 230).
CREATE TABLE IF NOT EXISTS maidan_thread_results (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    result JSONB NOT NULL,
    produced_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    produced_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
