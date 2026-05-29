-- Cluster 39: 1:1 direct message conversations (thread-backed).

CREATE TABLE maidan_dm_conversations (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_low_id   UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    member_high_id  UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id       UUID NOT NULL UNIQUE REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, member_low_id, member_high_id),
    CHECK (member_low_id < member_high_id)
);

CREATE INDEX idx_dm_workspace_low ON maidan_dm_conversations (workspace_id, member_low_id);
CREATE INDEX idx_dm_workspace_high ON maidan_dm_conversations (workspace_id, member_high_id);
