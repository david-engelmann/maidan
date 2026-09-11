-- Cluster 371 (Wave 2 #19, G19/T3): a secret store (SQLite twin of pg 0076). The
-- value is AEAD-encrypted at rest; the event log holds only a `secret://<name>`
-- reference, never the value. Only the ciphertext is stored here.
CREATE TABLE IF NOT EXISTS maidan_secrets (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    value_ciphertext  TEXT NOT NULL,
    created_by        TEXT NOT NULL REFERENCES maidan_members(id),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (workspace_id, name)
);
