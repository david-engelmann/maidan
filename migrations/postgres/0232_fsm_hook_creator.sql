-- An FSM hook runs as the credential that registered it, and stops when that
-- credential does. A hook ran with the server's own authority, so a hook's
-- creator could reach tools it could not call itself, and a hook kept firing
-- after its creator's token was revoked or its member was deactivated.
--
-- Hooks registered before this column have no creator and never fire again;
-- an administrator re-registers the ones still wanted. Deleting the token row
-- leaves the hook without a creator, which stops it the same way.
ALTER TABLE maidan_fsm_hooks
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
