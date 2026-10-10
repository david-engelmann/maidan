-- How long an `approval_decide` confirmation link lives, per workspace. See
-- the Postgres twin.
ALTER TABLE maidan_approval_policies
    ADD COLUMN confirm_link_ttl_seconds INTEGER NOT NULL DEFAULT 600
    CHECK (confirm_link_ttl_seconds BETWEEN 60 AND 3600);
