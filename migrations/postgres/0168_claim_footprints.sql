-- Claim footprints (maidan_types::footprint). A thread declares what part of a
-- repository its work touches; while the thread has a live claim, the claim
-- queue skips any thread whose footprint would overlap it, and a footprint set
-- on a claimed thread that overlaps another live claim is refused. A footprint
-- frees itself when the claim ends, because "live" is read from the thread.
--
-- One row per item. `match_namespace` and `match_key` are what overlap compares: lane,
-- crate and branch by equal key, path and glob (one `path` namespace) by key
-- prefix, a glob keyed by what precedes its first wildcard.
CREATE TABLE maidan_thread_footprints (
    thread_id    UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
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
