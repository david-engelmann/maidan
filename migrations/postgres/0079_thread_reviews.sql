-- Cluster 375 (Wave 2 #22, G5/G-dev-5): required reviewers. A thread declares a
-- review requirement (k approvals) from a named reviewer set (n); a reviewer
-- submits an approve / request-changes decision. The FSM close-gate (375.2) then
-- requires k distinct approvals from reviewers that are NEITHER the owner nor the
-- assignee (separation of duties, Cluster 355), and no unresolved `refutes` edge.
-- A "poll/closer" this is NOT — it is a gate on `closed`.
CREATE TABLE IF NOT EXISTS maidan_thread_review_reqs (
    thread_id      UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    required_count BIGINT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The named reviewer set (the "n"). Empty = open review (any qualifying member).
CREATE TABLE IF NOT EXISTS maidan_thread_reviewers (
    thread_id  UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, member_id)
);

-- A reviewer's decision. One per (thread, reviewer) — re-submitting changes it.
CREATE TABLE IF NOT EXISTS maidan_thread_reviews (
    thread_id   UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reviewer_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    decision    TEXT NOT NULL,
    note        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, reviewer_id)
);
