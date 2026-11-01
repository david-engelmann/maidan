-- Intake: incoming work proposals that are not tasks yet. An agent, a human
-- or an integration proposes an item; a member triages it once. Accepting
-- links it to a thread (new or existing); rejecting records a reason.
CREATE TABLE IF NOT EXISTS maidan_intake_items (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT,
    source TEXT NOT NULL,
    proposed_by UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'proposed',
    thread_id UUID REFERENCES maidan_threads(id) ON DELETE SET NULL,
    triaged_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    triaged_at TIMESTAMPTZ,
    reject_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (title <> ''),
    CHECK (source IN ('agent', 'human', 'integration')),
    CHECK (state IN ('proposed', 'accepted', 'rejected')),
    CHECK (state <> 'rejected' OR (reject_reason IS NOT NULL AND reject_reason <> ''))
);

CREATE INDEX IF NOT EXISTS idx_intake_items_workspace_state
    ON maidan_intake_items (workspace_id, state, id);
