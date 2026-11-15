-- An OAuth grant names the RFC 8707 resource the member consented for, and a
-- token minted under it answers on that MCP endpoint alone: the full catalog
-- at /mcp/streamable, or the coordinator profile at /mcp/coordinator, the
-- smaller surface a directory that lists a fixed tool set connects to.
--
-- Every grant before this one was for /mcp/streamable, and its refresh
-- tokens carry that URL under the origin it was issued at. A grant with no
-- refresh token has no live token to keep, so it is revoked rather than
-- listed with no resource: every live grant names a real endpoint.
ALTER TABLE oauth_grants ADD COLUMN resource TEXT NOT NULL DEFAULT '';

UPDATE oauth_grants
SET resource = COALESCE(
    (SELECT r.resource FROM oauth_refresh_tokens r
     WHERE r.grant_id = oauth_grants.id
     ORDER BY r.created_at DESC
     LIMIT 1),
    '');

UPDATE oauth_grants SET revoked_at = NOW()
WHERE resource = '' AND revoked_at IS NULL;

ALTER TABLE oauth_grants ALTER COLUMN resource DROP DEFAULT;

-- One live grant per (client, member, workspace, scope, resource): a client
-- connected to both endpoints holds a grant for each.
DROP INDEX idx_oauth_grants_live_unique;
CREATE UNIQUE INDEX idx_oauth_grants_live_unique
    ON oauth_grants (client_id, member_id, workspace_id, scope, resource)
    WHERE revoked_at IS NULL;
