-- FSM automation hooks on thread state transitions (Cluster 52.0).

CREATE TABLE maidan_fsm_hooks (
    id                 UUID PRIMARY KEY,
    workspace_id       UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    label              TEXT,
    from_state         TEXT,
    to_state           TEXT,
    handler_kind       TEXT NOT NULL,
    handler_target     TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL DEFAULT '',
    enabled            BOOLEAN NOT NULL DEFAULT TRUE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at         TIMESTAMPTZ
);

CREATE INDEX idx_fsm_hooks_workspace
    ON maidan_fsm_hooks (workspace_id)
    WHERE revoked_at IS NULL;
