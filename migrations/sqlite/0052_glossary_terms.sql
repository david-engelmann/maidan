-- Cluster 321 (mirror of postgres 0053): a workspace's shared glossary — canonical
-- term -> definition (+ aliases as a JSON array stored in TEXT), so agents use words
-- the same way. One entry per (workspace, term); upsert on re-set. Flat by design.
-- Zero-blast-radius foundation — no routes/worker yet.
CREATE TABLE IF NOT EXISTS maidan_glossary_terms (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    term TEXT NOT NULL,
    definition TEXT NOT NULL,
    aliases TEXT NOT NULL DEFAULT '[]',
    created_by TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (workspace_id, term),
    CHECK (term <> '')
);

CREATE INDEX IF NOT EXISTS idx_glossary_workspace ON maidan_glossary_terms (workspace_id);
