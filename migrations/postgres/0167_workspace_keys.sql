-- Per-tenant key custody (Open Work Next 25). Each workspace has a key of its
-- own, stored here wrapped by the instance KEK (`wrapping_key_id` is that
-- KEK's fingerprint), and its message data keys are wrapped by it rather than
-- by the instance KEK directly. One workspace's key opens nothing of
-- another's; erasing a workspace deletes its keys; rotating a workspace's key
-- rewraps only its data-key rows, never the sealed messages.
--
-- One key per workspace is active (`retired_at IS NULL`). A rotation retires
-- it and adds a new one; the rewrap pass moves the data keys over, then
-- destroys the retired key (its wrapped bytes erased, `destroyed_at` set).
-- `custody` names who wraps the key: `local` (the instance KEK) today, a KMS
-- later.
CREATE TABLE maidan_workspace_keys (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    custody         TEXT NOT NULL,
    wrapping_key_id TEXT,
    wrapped_key     BYTEA,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at      TIMESTAMPTZ,
    destroyed_at    TIMESTAMPTZ,
    CHECK ((destroyed_at IS NULL) = (wrapped_key IS NOT NULL AND wrapping_key_id IS NOT NULL)),
    CHECK (destroyed_at IS NULL OR retired_at IS NOT NULL)
);
CREATE UNIQUE INDEX idx_workspace_keys_active ON maidan_workspace_keys (workspace_id)
    WHERE retired_at IS NULL;
CREATE INDEX idx_workspace_keys_wrapping ON maidan_workspace_keys (wrapping_key_id)
    WHERE destroyed_at IS NULL;

-- The workspace key that wraps a data key; NULL for a data key the instance
-- KEK wrapped directly, written before this migration, until the rewrap pass
-- moves it.
ALTER TABLE maidan_content_keys
    ADD COLUMN workspace_key_id UUID REFERENCES maidan_workspace_keys(id);
CREATE INDEX idx_content_keys_workspace_key ON maidan_content_keys (workspace_key_id)
    WHERE shredded_at IS NULL;
