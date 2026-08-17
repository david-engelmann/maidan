-- Cluster 231 (Program B, Arc E): the skills a task (thread) requires. Skill
-- routing: a task is claimable by a member only if every required skill is one
-- the member has declared (Cluster 230 `maidan_member_skills`) — set containment.
-- A task with no rows here has no requirement and is claimable by anyone.
CREATE TABLE IF NOT EXISTS maidan_thread_required_skills (
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    skill TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, skill),
    CHECK (skill <> '')
);

CREATE INDEX IF NOT EXISTS idx_thread_required_skills_skill
    ON maidan_thread_required_skills (skill);
