-- A thread closed by close-abandoned (duplicate, abandoned, superseded)
-- reached `closed` without landing. A row here keeps its dependents
-- waiting: a dependency counts as met only when it is closed or archived
-- and has no row here. Presence = closed without landing.
CREATE TABLE maidan_thread_unlanded_closes (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN ('duplicate', 'abandoned', 'superseded')),
    closed_by UUID NOT NULL REFERENCES maidan_members(id),
    closed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
