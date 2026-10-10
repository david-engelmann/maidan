-- How long a confirmation link from `approval_decide` lives, in seconds. It
-- was ten minutes, compiled in; a workspace now sets it on its approval
-- policy, from one minute to an hour. A row written before this keeps ten
-- minutes, as does a workspace with no row. A link already sent keeps the
-- lifetime it was minted with.
ALTER TABLE maidan_approval_policies
    ADD COLUMN confirm_link_ttl_seconds INTEGER NOT NULL DEFAULT 600
    CHECK (confirm_link_ttl_seconds BETWEEN 60 AND 3600);
