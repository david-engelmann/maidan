-- Cluster 375 (Wave 2 #22, G5/G-dev-5): required reviewers (SQLite twin of pg
-- 0079). A thread declares a review requirement (k approvals) from a named
-- reviewer set (n); a reviewer submits an approve / request-changes decision.
-- The FSM close-gate (375.2) requires k distinct approvals from reviewers that
-- are neither owner nor assignee (SoD), and no unresolved `refutes` edge.
CREATE TABLE IF NOT EXISTS maidan_thread_review_reqs (
    thread_id      TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    required_count INTEGER NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS maidan_thread_reviewers (
    thread_id  TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id  TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, member_id)
);

CREATE TABLE IF NOT EXISTS maidan_thread_reviews (
    thread_id   TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reviewer_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    decision    TEXT NOT NULL,
    note        TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (thread_id, reviewer_id)
);
