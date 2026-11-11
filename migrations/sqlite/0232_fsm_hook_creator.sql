-- An FSM hook runs as the credential that registered it; see the Postgres
-- twin (0232).
ALTER TABLE maidan_fsm_hooks
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
