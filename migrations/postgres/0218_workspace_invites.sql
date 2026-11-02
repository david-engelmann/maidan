-- Invites that bring a person into an existing workspace as a human member.
-- An invite is bound to one email and one role, stored only as a SHA-256 of
-- its secret, expires, and is consumed by the one sign-in that accepts it.
-- `role` on an accepted row is the role the member holds: a `viewer` keeps
-- only the read capabilities on a browser session.
CREATE TABLE maidan_workspace_invites (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    email_lower TEXT NOT NULL CHECK (email_lower = lower(email_lower) AND length(email_lower) BETWEEN 3 AND 320),
    role TEXT NOT NULL CHECK (role IN ('viewer', 'member')),
    created_by UUID NOT NULL REFERENCES maidan_members(id),
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    accepted_member_id UUID UNIQUE REFERENCES maidan_members(id) ON DELETE CASCADE,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    CHECK (expires_at > created_at),
    CHECK ((accepted_at IS NULL) = (accepted_member_id IS NULL)),
    CHECK (accepted_at IS NULL OR revoked_at IS NULL)
);
CREATE INDEX idx_workspace_invites_workspace_created
    ON maidan_workspace_invites (workspace_id, created_at DESC);

-- The invite a sign-in started from. Re-checked in the accepting
-- transaction, so it needs no foreign key on this short-lived row.
ALTER TABLE maidan_oidc_pending ADD COLUMN workspace_invite_id UUID;
