-- Hosted console sign-up: a pending OIDC flow that creates a workspace.
--
-- Adds the 'signup' kind to maidan_oidc_pending. A signup `state` can never
-- complete a sign-in or front-door flow, or the other way round (CHECK
-- constraint). The workspace_id stays NULL for signup; the callback creates
-- the workspace, admin member, identity row, and session in one transaction.
--
-- Sign-up runs only with MAIDAN_SIGNUP=1 (off by default). See
-- docs/Hosted Console.md.

ALTER TABLE maidan_oidc_pending
    DROP CONSTRAINT IF EXISTS maidan_oidc_pending_kind_check;

ALTER TABLE maidan_oidc_pending
    ADD CONSTRAINT maidan_oidc_pending_kind_check
    CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL) OR
        (kind = 'front_door' AND workspace_id IS NULL) OR
        (kind = 'signup' AND workspace_id IS NULL)
    );
