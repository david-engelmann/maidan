-- A share ticket records the API token that minted it; see the Postgres twin
-- (0244).
ALTER TABLE maidan_share_tickets
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
