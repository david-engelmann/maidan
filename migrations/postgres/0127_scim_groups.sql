-- SCIM 2.0 Groups (RFC 7643 §4.2): an identity provider's named set of the
-- users it provisioned into this workspace. Nothing else in Maidan is a named
-- set of members apart from a channel, and a channel carries messages and
-- access, so an IdP group is its own record.
--
-- A group and each of its members carry the workspace, and the membership row
-- references both through (id, workspace_id): a member of another workspace,
-- or a member the IdP did not provision, cannot be written into a group even
-- by a query that forgot to check. Deprovisioning a user (removing its
-- maidan_scim_users row) removes it from every group.
CREATE UNIQUE INDEX IF NOT EXISTS idx_scim_users_member_workspace
    ON maidan_scim_users (member_id, workspace_id);

CREATE TABLE IF NOT EXISTS maidan_scim_groups (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL CHECK (display_name <> ''),
    external_id  TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, workspace_id)
);
CREATE INDEX IF NOT EXISTS idx_scim_groups_workspace
    ON maidan_scim_groups (workspace_id, created_at);

CREATE TABLE IF NOT EXISTS maidan_scim_group_members (
    group_id     UUID NOT NULL,
    member_id    UUID NOT NULL,
    workspace_id UUID NOT NULL,
    added_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (group_id, member_id),
    FOREIGN KEY (group_id, workspace_id)
        REFERENCES maidan_scim_groups (id, workspace_id) ON DELETE CASCADE,
    FOREIGN KEY (member_id, workspace_id)
        REFERENCES maidan_scim_users (member_id, workspace_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_scim_group_members_member
    ON maidan_scim_group_members (member_id);
