-- Next 17 part two: `approval_decide`. See the Postgres twin.
ALTER TABLE maidan_approval_gates
    ADD COLUMN risk TEXT NOT NULL DEFAULT 'high' CHECK (risk IN ('low', 'medium', 'high'));
ALTER TABLE maidan_approval_gates ADD COLUMN decided_via_client TEXT;
ALTER TABLE maidan_approval_gates ADD COLUMN decided_via_client_version TEXT;
ALTER TABLE maidan_approval_gates
    ADD COLUMN model_asked INTEGER NOT NULL DEFAULT 0 CHECK (model_asked IN (0, 1));

CREATE TABLE maidan_approval_policies (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    confirm_at   TEXT NOT NULL CHECK (confirm_at IN ('low', 'medium', 'high')),
    updated_at   TEXT NOT NULL
);

CREATE TABLE maidan_approval_confirmations (
    gate_id        TEXT NOT NULL REFERENCES maidan_approval_gates(id) ON DELETE CASCADE,
    member_id      TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id   TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    actor_id       TEXT,
    nonce          TEXT NOT NULL,
    token_hash     TEXT NOT NULL UNIQUE,
    client_name    TEXT,
    client_version TEXT,
    note           TEXT,
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    used_at        TEXT,
    PRIMARY KEY (gate_id, member_id)
);
CREATE INDEX idx_approval_confirmations_workspace
    ON maidan_approval_confirmations (workspace_id, expires_at);
