-- The front door: a sign-in that starts without a workspace id (Hosted
-- Console, open questions 4 and 5).
--
-- A pending sign-in now has a kind. `sign_in` names the workspace the person
-- asked for; `front_door` names none, and the callback picks the identity's
-- most recently used workspace. The CHECK ties the two together, so a
-- front-door `state` can never complete a workspace sign-in or the other way
-- round.
--
-- The workspace id loses its foreign key. Login no longer looks the workspace
-- up before redirecting, so an unauthenticated caller cannot tell a real id
-- from an unknown one; the callback checks the workspace instead. Pending rows
-- live for minutes, so nothing is lost when a deleted workspace leaves one
-- behind: its callback finds no workspace and refuses.
ALTER TABLE maidan_oidc_pending
    DROP CONSTRAINT IF EXISTS maidan_oidc_pending_workspace_id_fkey;
ALTER TABLE maidan_oidc_pending
    ALTER COLUMN workspace_id DROP NOT NULL;
ALTER TABLE maidan_oidc_pending
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'sign_in';
ALTER TABLE maidan_oidc_pending
    ADD CONSTRAINT maidan_oidc_pending_kind_workspace CHECK (
        (kind = 'sign_in' AND workspace_id IS NOT NULL)
        OR (kind = 'front_door' AND workspace_id IS NULL)
    );

-- The front door finds an identity's workspaces by issuer and subject alone.
CREATE INDEX idx_oidc_identities_subject ON maidan_oidc_identities (issuer, subject);
