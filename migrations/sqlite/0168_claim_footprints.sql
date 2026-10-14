-- Claim footprints; see the Postgres twin (0168).
CREATE TABLE maidan_thread_footprints (
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    mode         TEXT NOT NULL CHECK (mode IN ('exclusive', 'shared')),
    kind         TEXT NOT NULL CHECK (kind IN ('path', 'glob', 'crate', 'lane', 'branch')),
    value        TEXT NOT NULL,
    match_namespace TEXT NOT NULL,
    match_key       TEXT NOT NULL,
    PRIMARY KEY (thread_id, kind, value)
);
CREATE INDEX idx_thread_footprints_overlap
    ON maidan_thread_footprints (workspace_id, repo, match_namespace, match_key);
