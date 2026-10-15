-- Sign-up pending rows (Next 26).
--
-- A pending sign-up (`kind = 'sign_up'`) names no workspace — the callback
-- creates one — and may carry the waitlist entry whose invite gates it.
-- The kind CHECK grows the third shape beside 0163's two, so a sign-up
-- `state` can never complete a sign-in or the other way round.
ALTER TABLE maidan_oidc_pending
    ADD COLUMN invite_entry_id UUID;
ALTER TABLE maidan_oidc_pending
    DROP CONSTRAINT maidan_oidc_pending_kind_workspace;
ALTER TABLE maidan_oidc_pending
    ADD CONSTRAINT maidan_oidc_pending_kind_workspace CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL)
        OR (kind = 'front_door' AND workspace_id IS NULL)
        OR (kind = 'sign_up' AND workspace_id IS NULL)
    );
