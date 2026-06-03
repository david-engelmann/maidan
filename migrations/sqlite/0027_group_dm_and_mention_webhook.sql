-- Cluster 97: multi-member group DM conversations (thread-backed).
-- Cluster 98: per-workspace mention notification webhook route.

CREATE TABLE maidan_group_dm_conversations (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id       TEXT NOT NULL UNIQUE REFERENCES maidan_threads(id) ON DELETE CASCADE,
    title           TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE maidan_group_dm_members (
    group_dm_id     TEXT NOT NULL REFERENCES maidan_group_dm_conversations(id) ON DELETE CASCADE,
    member_id       TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (group_dm_id, member_id)
);

CREATE INDEX idx_group_dm_workspace ON maidan_group_dm_conversations (workspace_id);
CREATE INDEX idx_group_dm_member ON maidan_group_dm_members (member_id);

ALTER TABLE maidan_workspaces ADD COLUMN mention_webhook_id TEXT REFERENCES maidan_webhook_subscriptions(id) ON DELETE SET NULL;
