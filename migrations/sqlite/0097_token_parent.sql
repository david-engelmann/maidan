-- Cluster 401.3: a derived token knows its parent, so revoking the parent
-- reaches it.
--
-- Cluster 395 added holder-side attenuation and Cluster 397.7 made a derived
-- token inherit the parent's app installation and per-token quotas — closing
-- two ways to shed a bound by re-issuing. The third stayed open: the parent
-- link was recorded only in *audit metadata*, so revocation could not traverse
-- and a child outlived the credential it was minted from.
--
-- `ON DELETE SET NULL`, not CASCADE: deleting a parent row should sever the
-- link, never delete its children. Workspace teardown already cascades through
-- `workspace_id`.
ALTER TABLE maidan_api_tokens
    ADD COLUMN parent_token_id TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;

-- The cascade walks children-of, so index that direction.
CREATE INDEX IF NOT EXISTS idx_api_tokens_parent
    ON maidan_api_tokens (parent_token_id);
