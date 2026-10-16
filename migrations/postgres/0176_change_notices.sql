-- Change notices (crates/maidan-server/src/change_sweep.rs). A round records
-- the files its diff touched, so a merge can be matched against the
-- footprints of the claims still working in the same repository, and the
-- branch it merges into, which the notice names.
ALTER TABLE maidan_change_rounds
    ADD COLUMN paths JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN base_branch TEXT;

-- `noticed_at` makes a merge notify once, even with two replicas sweeping.
ALTER TABLE maidan_change_pulls ADD COLUMN noticed_at TIMESTAMPTZ;
-- A merge recorded before rounds had paths has nothing to match on, and its
-- claims have long since moved on.
UPDATE maidan_change_pulls SET noticed_at = merged_at WHERE merged_at IS NOT NULL;

-- One row per notice a thread was given. `dedupe_key` names what it was
-- about (a merged pull request, a delivered commit), so the same news is
-- never said to a thread twice.
CREATE TABLE maidan_thread_notices (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id    UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    repo         TEXT NOT NULL,
    pull_number  BIGINT NOT NULL,
    paths        JSONB NOT NULL DEFAULT '[]'::jsonb,
    base_branch  TEXT,
    base_behind  BIGINT,
    dedupe_key   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL,
    UNIQUE (thread_id, dedupe_key)
);
CREATE INDEX idx_thread_notices_thread ON maidan_thread_notices (thread_id, created_at DESC);
