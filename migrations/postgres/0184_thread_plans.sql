-- Planner fan-out (Next 33, #1421): a plan parent's children, submitted in
-- one call by the planner holding the parent's claim.
--
-- The lease is kept so a retry from the same claim with the same keys can be
-- answered with the children it already made instead of a conflict. A child's
-- key is the planner's name for it, unique within its plan, and is what
-- `depends_on` referred to. Rows die with their threads.
CREATE TABLE maidan_thread_plans (
    parent_thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    lease_id         UUID NOT NULL,
    submitted_by     UUID NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL
);
CREATE TABLE maidan_thread_plan_children (
    parent_thread_id UUID NOT NULL REFERENCES maidan_thread_plans(parent_thread_id) ON DELETE CASCADE,
    child_key        TEXT NOT NULL,
    child_thread_id  UUID NOT NULL UNIQUE REFERENCES maidan_threads(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    PRIMARY KEY (parent_thread_id, child_key)
);
