-- Stateless MCP resource subscriptions. See the Postgres twin. Times are
-- millisecond `...Z` text, compared as strings.
CREATE TABLE IF NOT EXISTS maidan_mcp_resource_subscriptions (
    subscriber TEXT NOT NULL,
    uri TEXT NOT NULL,
    workspace_id TEXT REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (subscriber, uri)
);
CREATE INDEX IF NOT EXISTS maidan_mcp_resource_subscriptions_uri
    ON maidan_mcp_resource_subscriptions (uri);
CREATE INDEX IF NOT EXISTS maidan_mcp_resource_subscriptions_expires_at
    ON maidan_mcp_resource_subscriptions (expires_at);
