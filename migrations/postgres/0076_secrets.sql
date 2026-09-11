-- Cluster 371 (Wave 2 #19, G19/T3): a secret store. A workspace names a secret;
-- the value is AEAD-encrypted at rest (the Cluster-189 keyring) and NEVER leaves
-- in the event log — the log holds a `secret://<name>` reference, this table holds
-- the value, and Pi resolves it at exec (or a broker substitutes it on egress to
-- an allowlisted host). Only the ciphertext is stored; encryption/decryption lives
-- in the route layer, which holds the key.
CREATE TABLE IF NOT EXISTS maidan_secrets (
    id                UUID PRIMARY KEY,
    workspace_id      UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    value_ciphertext  TEXT NOT NULL,
    created_by        UUID NOT NULL REFERENCES maidan_members(id),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, name)
);
