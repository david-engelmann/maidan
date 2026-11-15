-- An OAuth grant names the resource it was given for. See the Postgres twin
-- (0239) for why. SQLite cannot drop a column default, so '' stays the
-- default here; the store always writes the resource.
ALTER TABLE oauth_grants ADD COLUMN resource TEXT NOT NULL DEFAULT '';

UPDATE oauth_grants
SET resource = COALESCE(
    (SELECT r.resource FROM oauth_refresh_tokens r
     WHERE r.grant_id = oauth_grants.id
     ORDER BY r.created_at DESC
     LIMIT 1),
    '');

DROP INDEX idx_oauth_grants_live_unique;
CREATE UNIQUE INDEX idx_oauth_grants_live_unique
    ON oauth_grants (client_id, member_id, workspace_id, scope, resource)
    WHERE revoked_at IS NULL;
