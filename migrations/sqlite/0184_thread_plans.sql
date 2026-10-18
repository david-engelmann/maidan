-- Planner fan-out (Next 33, #1421); see the Postgres twin (0184).
CREATE TABLE maidan_thread_plans (
    parent_thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    lease_id         TEXT NOT NULL,
    submitted_by     TEXT NOT NULL,
    created_at       TEXT NOT NULL
);
CREATE TABLE maidan_thread_plan_children (
    parent_thread_id TEXT NOT NULL REFERENCES maidan_thread_plans(parent_thread_id) ON DELETE CASCADE,
    child_key        TEXT NOT NULL,
    child_thread_id  TEXT NOT NULL UNIQUE REFERENCES maidan_threads(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    PRIMARY KEY (parent_thread_id, child_key)
);
