-- Cluster 231 (mirror of postgres 0040): the skills a task requires. Skill
-- routing matches these against a member's declared skills (Cluster 230);
-- a task with no rows is claimable by anyone.
CREATE TABLE IF NOT EXISTS maidan_thread_required_skills (
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    skill TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (thread_id, skill),
    CHECK (skill <> '')
);

CREATE INDEX IF NOT EXISTS idx_thread_required_skills_skill
    ON maidan_thread_required_skills (skill);
