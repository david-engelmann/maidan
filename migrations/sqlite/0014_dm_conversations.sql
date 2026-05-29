-- Cluster 39: 1:1 direct message conversations (thread-backed).

CREATE TABLE maidan_dm_conversations (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_low_id   TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    member_high_id  TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    thread_id       TEXT NOT NULL UNIQUE REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (workspace_id, member_low_id, member_high_id),
    CHECK (member_low_id < member_high_id)
);

CREATE INDEX idx_dm_workspace_low ON maidan_dm_conversations (workspace_id, member_low_id);
CREATE INDEX idx_dm_workspace_high ON maidan_dm_conversations (workspace_id, member_high_id);
