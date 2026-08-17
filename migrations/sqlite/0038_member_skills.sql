-- Cluster 230 (mirror of postgres 0039): capability registry — a member's
-- declared skill tags. Skill routing matches a task's required skills against
-- these. Zero-blast-radius foundation (no worker/routes yet).
CREATE TABLE IF NOT EXISTS maidan_member_skills (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    skill TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, skill),
    CHECK (skill <> '')
);

CREATE INDEX IF NOT EXISTS idx_member_skills_skill ON maidan_member_skills (skill);
