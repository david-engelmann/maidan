-- MCP-002: grant `member:self` to every stored capability list that holds
-- `workspace:read`. See the Postgres twin (0237) for why; this is the same
-- one-time data fix. The entry is appended to the serialized array, and a
-- list that already holds `member:self` is left alone.

-- Tokens, the authority a request carries.
UPDATE maidan_api_tokens
SET capabilities = substr(rtrim(capabilities), 1, length(rtrim(capabilities)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(maidan_api_tokens.capabilities) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(maidan_api_tokens.capabilities) WHERE value = 'member:self');

-- What a delegated token or an app's token may later be minted with.
UPDATE maidan_delegation_grants
SET capabilities = substr(rtrim(capabilities), 1, length(rtrim(capabilities)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(maidan_delegation_grants.capabilities) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(maidan_delegation_grants.capabilities) WHERE value = 'member:self');

UPDATE maidan_app_installations
SET granted_capabilities = substr(rtrim(granted_capabilities), 1, length(rtrim(granted_capabilities)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(maidan_app_installations.granted_capabilities) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(maidan_app_installations.granted_capabilities) WHERE value = 'member:self');

-- OAuth: the scope a refresh mints from, a client's ceiling, and the
-- requests and codes still in flight.
UPDATE oauth_grants
SET scope = substr(rtrim(scope), 1, length(rtrim(scope)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(oauth_grants.scope) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(oauth_grants.scope) WHERE value = 'member:self');

UPDATE oauth_clients
SET allowed_scopes = substr(rtrim(allowed_scopes), 1, length(rtrim(allowed_scopes)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(oauth_clients.allowed_scopes) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(oauth_clients.allowed_scopes) WHERE value = 'member:self');

UPDATE oauth_authorization_codes
SET scope = substr(rtrim(scope), 1, length(rtrim(scope)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(oauth_authorization_codes.scope) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(oauth_authorization_codes.scope) WHERE value = 'member:self');

UPDATE oauth_pending_requests
SET scope = substr(rtrim(scope), 1, length(rtrim(scope)) - 1) || ',"member:self"]'
WHERE EXISTS (SELECT 1 FROM json_each(oauth_pending_requests.scope) WHERE value = 'workspace:read')
  AND NOT EXISTS (SELECT 1 FROM json_each(oauth_pending_requests.scope) WHERE value = 'member:self');
