-- Next 17 part two: `approval_decide`, the model-callable way to answer a gate.
--
-- `risk` is how much a wrong accept would cost, set by whoever opens the gate.
-- `high` is the default, so a gate that says nothing is treated as the
-- costliest kind. The decision columns say who-through-what decided: the MCP
-- client a model used (as it named itself, which proves nothing) and whether a
-- model asked at all. A REST or console answer leaves them empty and false.
ALTER TABLE maidan_approval_gates
    ADD COLUMN risk TEXT NOT NULL DEFAULT 'high' CHECK (risk IN ('low', 'medium', 'high')),
    ADD COLUMN decided_via_client TEXT,
    ADD COLUMN decided_via_client_version TEXT,
    ADD COLUMN model_asked BOOLEAN NOT NULL DEFAULT FALSE;

-- The lowest gate risk at which a model's accept needs a person to confirm it
-- in the console. No row means `low`: every accept through the tool needs a
-- confirmation until an admin says otherwise. Set with token:admin.
CREATE TABLE maidan_approval_policies (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    confirm_at   TEXT NOT NULL CHECK (confirm_at IN ('low', 'medium', 'high')),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- A pending confirmation: a model asked to accept `gate_id` on `member_id`'s
-- credential, and that person has to confirm it in the console. One row per
-- gate and member, so a looping model gets the live link back instead of a
-- new one. The link's token is derived from `nonce` with the server's secret,
-- and only its SHA-256 is kept. `used_at` makes it single-use.
CREATE TABLE maidan_approval_confirmations (
    gate_id        UUID NOT NULL REFERENCES maidan_approval_gates(id) ON DELETE CASCADE,
    member_id      UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    workspace_id   UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    actor_id       UUID,
    nonce          UUID NOT NULL,
    token_hash     TEXT NOT NULL UNIQUE,
    client_name    TEXT,
    client_version TEXT,
    note           TEXT,
    created_at     TIMESTAMPTZ NOT NULL,
    expires_at     TIMESTAMPTZ NOT NULL,
    used_at        TIMESTAMPTZ,
    PRIMARY KEY (gate_id, member_id)
);
CREATE INDEX idx_approval_confirmations_workspace
    ON maidan_approval_confirmations (workspace_id, expires_at);
