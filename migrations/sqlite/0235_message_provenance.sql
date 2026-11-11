-- Where a bridged message came from, and the bridge ingress key; see the
-- Postgres twin (0235).
ALTER TABLE maidan_messages ADD COLUMN origin TEXT
    CHECK (origin IN ('github', 'slack'));
ALTER TABLE maidan_messages ADD COLUMN origin_ref TEXT;

CREATE TABLE maidan_bridge_receipts (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    origin TEXT NOT NULL CHECK (origin IN ('github', 'slack')),
    external_id TEXT NOT NULL,
    received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (workspace_id, origin, external_id)
);
