-- Open Work Next 5 (P1-6): agent self-reported status.
-- `declare_status` lets the claim holder or owner say what the agent is doing.
-- Presence = a declaration is active; absence = no declaration (cleared).
-- `stalled` is system-computed only and is not a declarable value.
CREATE TABLE maidan_thread_status (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('working', 'needs_input', 'needs_review', 'blocked', 'done')),
    note TEXT NOT NULL,
    declared_by UUID NOT NULL REFERENCES maidan_members(id),
    declared_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
