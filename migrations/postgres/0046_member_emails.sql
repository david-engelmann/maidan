-- Cluster 248 (Program C, Arc I): a member's delivery email address. Where email
-- notifications go (Cluster 247's SMTP transport). One row per member (a separate
-- table, not a column on maidan_members, so the shared member row-mapping is
-- untouched). The zero-blast-radius foundation for email delivery — no wiring yet.
CREATE TABLE IF NOT EXISTS maidan_member_emails (
    member_id UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
