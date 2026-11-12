-- MCP-002: the writes to a member's own personal state (follows, mutes,
-- notification read / snooze / preferences, delivery mode, the member's email
-- and the inbox read mark) moved from `workspace:read` to the new
-- `member:self` capability.
--
-- Capabilities are stored expanded, as a JSON array, when a token, grant or
-- installation is made, and a request reads them back as stored. So every
-- stored list that holds `workspace:read` could make these writes until now,
-- and would lose them the moment the new check ships. This one-time data fix
-- grants `member:self` to each such list, so nothing that worked before stops.
-- Lists made after it come from presets that already carry `member:self`.
--
-- The new entry is appended to the serialized array, so a list keeps its
-- order and its compact form. A list that already holds `member:self` is left
-- alone, so the statement is safe to run twice.

-- Tokens, the authority a request carries.
UPDATE maidan_api_tokens
SET capabilities = regexp_replace(capabilities, '\]\s*$', ',"member:self"]')
WHERE capabilities::jsonb ? 'workspace:read'
  AND NOT capabilities::jsonb ? 'member:self';

-- What a delegated token or an app's token may later be minted with.
UPDATE maidan_delegation_grants
SET capabilities = regexp_replace(capabilities, '\]\s*$', ',"member:self"]')
WHERE capabilities::jsonb ? 'workspace:read'
  AND NOT capabilities::jsonb ? 'member:self';

UPDATE maidan_app_installations
SET granted_capabilities = regexp_replace(granted_capabilities, '\]\s*$', ',"member:self"]')
WHERE granted_capabilities::jsonb ? 'workspace:read'
  AND NOT granted_capabilities::jsonb ? 'member:self';

-- OAuth: the scope a refresh mints from, a client's ceiling, and the
-- requests and codes still in flight.
UPDATE oauth_grants
SET scope = regexp_replace(scope, '\]\s*$', ',"member:self"]')
WHERE scope::jsonb ? 'workspace:read'
  AND NOT scope::jsonb ? 'member:self';

UPDATE oauth_clients
SET allowed_scopes = regexp_replace(allowed_scopes, '\]\s*$', ',"member:self"]')
WHERE allowed_scopes::jsonb ? 'workspace:read'
  AND NOT allowed_scopes::jsonb ? 'member:self';

UPDATE oauth_authorization_codes
SET scope = regexp_replace(scope, '\]\s*$', ',"member:self"]')
WHERE scope::jsonb ? 'workspace:read'
  AND NOT scope::jsonb ? 'member:self';

UPDATE oauth_pending_requests
SET scope = regexp_replace(scope, '\]\s*$', ',"member:self"]')
WHERE scope::jsonb ? 'workspace:read'
  AND NOT scope::jsonb ? 'member:self';
