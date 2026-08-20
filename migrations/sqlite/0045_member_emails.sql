-- Cluster 248 (mirror of postgres 0046): a member's delivery email address (where
-- email notifications go). One row per member, a separate table so the shared member
-- row-mapping is untouched. Zero-blast-radius foundation — no wiring yet.
CREATE TABLE IF NOT EXISTS maidan_member_emails (
    member_id TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
