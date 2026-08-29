-- Cluster 321 (fidelity + context flagship arc): a workspace's shared glossary —
-- a canonical term -> definition (+ aliases), so agents use words the same way (the
-- anti-drift pin; the target of the `defines` reference relation from Cluster 319).
-- One entry per (workspace, term); upsert on re-set. Flat by design — no hierarchy
-- (that is where a glossary tips into a KG product). Zero-blast-radius foundation:
-- no routes/worker yet (the Cluster 159 / 217 / 234 pattern).
CREATE TABLE IF NOT EXISTS maidan_glossary_terms (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    term TEXT NOT NULL,
    definition TEXT NOT NULL,
    aliases JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, term),
    CHECK (term <> '')
);

CREATE INDEX IF NOT EXISTS idx_glossary_workspace ON maidan_glossary_terms (workspace_id);
