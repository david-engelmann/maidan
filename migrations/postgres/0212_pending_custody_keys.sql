-- #1545: the wrap journal. A row is committed, in its own transaction,
-- before the custody creates a key for a workspace key, and gets the key's
-- wrapping key id as soon as the custody returns it. The stray-key sweep
-- destroys only keys journalled here that no workspace key row refers to,
-- never a key merely missing from this database's rows: another deployment
-- may share the custody account. A row goes when its workspace key row
-- commits or its key is destroyed.
CREATE TABLE maidan_pending_custody_keys (
    workspace_key_id UUID PRIMARY KEY,
    wrapping_key_id TEXT,
    intended_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
