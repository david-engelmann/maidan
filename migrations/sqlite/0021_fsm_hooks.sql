-- FSM automation hooks on thread state transitions (Cluster 52.0).

CREATE TABLE maidan_fsm_hooks (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    label              TEXT,
    from_state         TEXT,
    to_state           TEXT,
    handler_kind       TEXT NOT NULL,
    handler_target     TEXT NOT NULL,
    secret_ciphertext  TEXT NOT NULL DEFAULT '',
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at         TEXT
);

CREATE INDEX idx_fsm_hooks_workspace
    ON maidan_fsm_hooks (workspace_id);
