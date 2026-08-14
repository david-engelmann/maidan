-- Cluster 217 (Program B, agentic orchestration): task-dependency DAG edges on
-- threads.
--
-- A thread (task) `thread_id` depends on `depends_on_thread_id`; it is "ready" to
-- claim only once every dependency reaches a terminal ThreadState (closed or
-- archived). Edges are directed (thread -> depends_on). ON DELETE CASCADE so
-- removing either thread drops the edge. A self-dependency is rejected by the
-- CHECK; transitive cycle *prevention* is deferred (a cyclic task simply never
-- becomes ready — it deadlocks rather than corrupts).
CREATE TABLE IF NOT EXISTS maidan_thread_dependencies (
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    depends_on_thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, depends_on_thread_id),
    CHECK (thread_id <> depends_on_thread_id)
);

-- Reverse lookups ("what is blocked by this task?") + the readiness join.
CREATE INDEX IF NOT EXISTS idx_thread_deps_depends_on
    ON maidan_thread_dependencies (depends_on_thread_id);
