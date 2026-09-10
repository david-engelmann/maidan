-- Cluster 366 (Wave 1 #14, SCIM-as-OIDC-P3): SCIM 2.0 user provisioning (SQLite
-- twin of pg 0072). Maps a SCIM User to a Maidan member, tracking externalId +
-- the SCIM active flag.
CREATE TABLE IF NOT EXISTS maidan_scim_users (
    member_id    TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    external_id  TEXT,
    active       INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scim_users_workspace ON maidan_scim_users (workspace_id);
