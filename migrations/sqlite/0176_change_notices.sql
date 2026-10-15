-- Change notices; see the Postgres twin (0176).
ALTER TABLE maidan_change_rounds ADD COLUMN paths TEXT NOT NULL DEFAULT '[]';
ALTER TABLE maidan_change_rounds ADD COLUMN base_branch TEXT;

ALTER TABLE maidan_change_pulls ADD COLUMN noticed_at TEXT;
UPDATE maidan_change_pulls SET noticed_at = merged_at WHERE merged_at IS NOT NULL;

CREATE TABLE maidan_thread_notices (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    repo         TEXT NOT NULL,
    pull_number  INTEGER NOT NULL,
    paths        TEXT NOT NULL DEFAULT '[]',
    base_branch  TEXT,
    base_behind  INTEGER,
    dedupe_key   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (thread_id, dedupe_key)
);
CREATE INDEX idx_thread_notices_thread ON maidan_thread_notices (thread_id, created_at DESC);
