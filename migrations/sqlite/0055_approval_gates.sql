-- Cluster 350 (mirror of postgres 0056): a durable human-approval gate (the held
-- gate). JSON columns (`schema`, `content`) are stored as TEXT. A human resolves a
-- `pending` gate to accept/decline/cancel; silence never resolves it. Zero-blast-
-- radius foundation — no tool/route change yet.
CREATE TABLE IF NOT EXISTS maidan_approval_gates (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id TEXT REFERENCES maidan_threads(id) ON DELETE CASCADE,
    requested_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    schema TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    content TEXT,
    resolved_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT
);

-- The queryable held-gate list is "pending gates in a workspace, oldest first".
CREATE INDEX IF NOT EXISTS idx_approval_gates_pending
    ON maidan_approval_gates (workspace_id, created_at);
