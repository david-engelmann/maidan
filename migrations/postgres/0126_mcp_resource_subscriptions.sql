-- Stateless MCP resource subscriptions (`resources/subscribe` over `POST /mcp`
-- and the stateless streamable POST). A stateless caller has no session, so
-- its credential is its identity and its subscribe and its listener may land
-- on different replicas; the subscription is kept here so that whichever
-- replica holds the listener can find it. `subscriber` spells the principal
-- (workspace, member, actor, token, app installation, grant). A row lapses at
-- `expires_at` unless a replica holding an open listener for its subscriber
-- keeps extending it. `workspace_id` and `member_id` are NULL only for an
-- auth-disabled caller, which has neither.
CREATE TABLE IF NOT EXISTS maidan_mcp_resource_subscriptions (
    subscriber TEXT NOT NULL,
    uri TEXT NOT NULL,
    workspace_id UUID REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id UUID,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (subscriber, uri)
);
CREATE INDEX IF NOT EXISTS maidan_mcp_resource_subscriptions_uri
    ON maidan_mcp_resource_subscriptions (uri);
CREATE INDEX IF NOT EXISTS maidan_mcp_resource_subscriptions_expires_at
    ON maidan_mcp_resource_subscriptions (expires_at);
