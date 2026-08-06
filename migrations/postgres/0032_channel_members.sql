-- Cluster 159 (RBAC part A): per-channel membership.
-- Additive only — no enforcement yet (Cluster 160 adds ensure_channel_access).
-- Public channels stay open to the workspace; private channels will be gated
-- to explicit members. `role` distinguishes plain members from channel admins.

CREATE TABLE maidan_channel_members (
    channel_id  UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    member_id   UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'admin')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel_id, member_id)
);

CREATE INDEX idx_channel_members_member ON maidan_channel_members (member_id);
