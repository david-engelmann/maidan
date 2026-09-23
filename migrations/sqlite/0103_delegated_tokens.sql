-- Cluster 411 (Postgres 0104 twin): bind exchanged tokens to their grant.
ALTER TABLE maidan_api_tokens
    ADD COLUMN delegation_grant_id TEXT REFERENCES maidan_delegation_grants(id);

CREATE INDEX IF NOT EXISTS idx_api_tokens_delegation_grant
    ON maidan_api_tokens (delegation_grant_id)
    WHERE delegation_grant_id IS NOT NULL;
