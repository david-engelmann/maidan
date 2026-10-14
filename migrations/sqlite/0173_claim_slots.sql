-- Claim slots; see the Postgres twin (0173). SQLite cannot alter a CHECK, so
-- the footprint table is rebuilt with `slot` among its kinds.
CREATE TABLE maidan_thread_footprints_next (
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    mode         TEXT NOT NULL CHECK (mode IN ('exclusive', 'shared')),
    kind         TEXT NOT NULL CHECK (kind IN ('path', 'glob', 'crate', 'lane', 'branch', 'slot')),
    value        TEXT NOT NULL,
    match_namespace TEXT NOT NULL,
    match_key       TEXT NOT NULL,
    PRIMARY KEY (thread_id, kind, value)
);
INSERT INTO maidan_thread_footprints_next
    SELECT thread_id, workspace_id, repo, mode, kind, value, match_namespace, match_key
    FROM maidan_thread_footprints;
DROP TABLE maidan_thread_footprints;
ALTER TABLE maidan_thread_footprints_next RENAME TO maidan_thread_footprints;
CREATE INDEX idx_thread_footprints_overlap
    ON maidan_thread_footprints (workspace_id, repo, match_namespace, match_key);
CREATE INDEX idx_thread_footprints_slot
    ON maidan_thread_footprints (workspace_id, match_namespace, match_key);

CREATE TABLE maidan_slot_capacities (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    slot         TEXT NOT NULL,
    capacity     INTEGER NOT NULL CHECK (capacity > 0),
    updated_at   TEXT NOT NULL,
    updated_by   TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    PRIMARY KEY (workspace_id, slot)
);
