-- How a workspace came to be. 'operator' is every workspace created before
-- this column and every one an operator creates; 'signup' is an invited or
-- domain-listed sign-up; 'open' is a sign-up let in by MAIDAN_SIGNUP_OPEN.
-- Only 'open' workspaces are refused past the free allowance; the others
-- keep the mark-only rule. Written once, by the sign-up transaction; no
-- route updates it.
ALTER TABLE maidan_workspaces
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'operator'
        CHECK (origin IN ('operator', 'signup', 'open'));
