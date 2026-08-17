-- Cluster 230 (Program B, agentic orchestration): capability registry — the
-- free-form skill tags a member (agent) declares it can do. Skill routing
-- (Cluster 231+) matches a task's required skills against these: a task is
-- claimable by a member iff its required skills are a subset of the member's.
-- No worker/routes in this cluster — the zero-blast-radius foundation pattern
-- (Cluster 159 / 217 / 226).
CREATE TABLE IF NOT EXISTS maidan_member_skills (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    skill TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, skill),
    CHECK (skill <> '')
);

-- Reverse lookup: which members hold a given skill (skill routing / discovery).
CREATE INDEX IF NOT EXISTS idx_member_skills_skill ON maidan_member_skills (skill);
