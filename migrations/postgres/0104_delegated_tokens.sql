-- Cluster 411: bind exchanged tokens to the durable grant that authorized them.
ALTER TABLE maidan_api_tokens
    ADD COLUMN delegation_grant_id UUID REFERENCES maidan_delegation_grants(id);

CREATE INDEX IF NOT EXISTS idx_api_tokens_delegation_grant
    ON maidan_api_tokens (delegation_grant_id)
    WHERE delegation_grant_id IS NOT NULL;
