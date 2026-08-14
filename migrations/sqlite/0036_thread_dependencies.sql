-- Cluster 217 (mirror of postgres 0037): task-dependency DAG edges on threads.
--
-- A thread (task) `thread_id` depends on `depends_on_thread_id`; it is "ready"
-- only once every dependency is terminal (closed/archived). Self-dependency is
-- rejected by the CHECK; transitive cycle prevention is deferred (a cyclic task
-- just never becomes ready).
CREATE TABLE IF NOT EXISTS maidan_thread_dependencies (
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    depends_on_thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (thread_id, depends_on_thread_id),
    CHECK (thread_id <> depends_on_thread_id)
);

CREATE INDEX IF NOT EXISTS idx_thread_deps_depends_on
    ON maidan_thread_dependencies (depends_on_thread_id);
