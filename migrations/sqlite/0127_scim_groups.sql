-- SCIM 2.0 Groups. See the Postgres twin: the membership row references the
-- group and the SCIM user through (id, workspace_id), so a member of another
-- workspace cannot be written into a group.
CREATE UNIQUE INDEX IF NOT EXISTS idx_scim_users_member_workspace
    ON maidan_scim_users (member_id, workspace_id);

CREATE TABLE IF NOT EXISTS maidan_scim_groups (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL CHECK (display_name <> ''),
    external_id  TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (id, workspace_id)
);
CREATE INDEX IF NOT EXISTS idx_scim_groups_workspace
    ON maidan_scim_groups (workspace_id, created_at);

CREATE TABLE IF NOT EXISTS maidan_scim_group_members (
    group_id     TEXT NOT NULL,
    member_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    added_at     TEXT NOT NULL,
    PRIMARY KEY (group_id, member_id),
    FOREIGN KEY (group_id, workspace_id)
        REFERENCES maidan_scim_groups (id, workspace_id) ON DELETE CASCADE,
    FOREIGN KEY (member_id, workspace_id)
        REFERENCES maidan_scim_users (member_id, workspace_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_scim_group_members_member
    ON maidan_scim_group_members (member_id);
