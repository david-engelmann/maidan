-- See the Postgres twin: a session made from a token keeps its id.
ALTER TABLE maidan_sessions
    ADD COLUMN api_token_id TEXT REFERENCES maidan_api_tokens(id) ON DELETE CASCADE;
CREATE INDEX idx_sessions_api_token ON maidan_sessions (api_token_id)
    WHERE api_token_id IS NOT NULL;

ALTER TABLE maidan_sessions DROP COLUMN csrf_secret;
