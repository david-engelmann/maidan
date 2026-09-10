-- Cluster 366 (Wave 1 #14, SCIM-as-OIDC-P3): SCIM 2.0 user provisioning. Maps a
-- SCIM User (RFC 7643) to a Maidan member, tracking the IdP's externalId and the
-- SCIM `active` flag. `userName` is the member handle and `id` is the member id;
-- this side table holds only the SCIM-specific fields so the members row is
-- untouched. Deactivation (active=false) / delete revokes the member's tokens.
CREATE TABLE IF NOT EXISTS maidan_scim_users (
    member_id    UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    external_id  TEXT,
    active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_scim_users_workspace ON maidan_scim_users (workspace_id);
