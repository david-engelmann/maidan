-- SQLite mirror of postgres 0215: a workspace's intake queue of incoming
-- work proposals. Accepting links a thread; rejecting records a reason.
CREATE TABLE IF NOT EXISTS maidan_intake_items (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT,
    source TEXT NOT NULL,
    proposed_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'proposed',
    thread_id TEXT REFERENCES maidan_threads(id) ON DELETE SET NULL,
    triaged_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    triaged_at TEXT,
    reject_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (title <> ''),
    CHECK (source IN ('agent', 'human', 'integration')),
    CHECK (state IN ('proposed', 'accepted', 'rejected')),
    CHECK (state <> 'rejected' OR (reject_reason IS NOT NULL AND reject_reason <> ''))
);

CREATE INDEX IF NOT EXISTS idx_intake_items_workspace_state
    ON maidan_intake_items (workspace_id, state, id);
