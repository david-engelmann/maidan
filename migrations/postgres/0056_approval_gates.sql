-- Cluster 350 (Phase XXV, the held gate): a durable, queryable human-approval
-- gate. An agent's `request_approval` opens one `pending` gate and returns an
-- `input-required` result instead of blocking on an in-memory oneshot; a human
-- later resolves it to accept/decline/cancel (silence never resolves it — there
-- is no timeout auto-approve). The gate is persisted so it survives a dropped
-- connection and can be listed while outstanding (queryable). `thread_id`
-- (nullable) attaches a gate to a thread for the N6 required-human claim gate in
-- a later PR. Zero-blast-radius foundation — no tool/route change in this PR
-- (the Cluster 159 / 234 pattern).
CREATE TABLE IF NOT EXISTS maidan_approval_gates (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id UUID REFERENCES maidan_threads(id) ON DELETE CASCADE,
    requested_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    schema JSONB,
    state TEXT NOT NULL DEFAULT 'pending',
    content JSONB,
    resolved_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ
);

-- The queryable held-gate list is "pending gates in a workspace, oldest first".
CREATE INDEX IF NOT EXISTS idx_approval_gates_pending
    ON maidan_approval_gates (workspace_id, created_at)
    WHERE state = 'pending';
