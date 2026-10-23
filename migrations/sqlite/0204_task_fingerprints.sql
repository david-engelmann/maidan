-- Task fingerprints (Context Economics C8); see the Postgres twin.
CREATE TABLE maidan_task_fingerprints (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads (id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL CHECK (length(fingerprint) = 64 AND fingerprint NOT GLOB '*[^0-9a-f]*'),
    duplicate_of TEXT REFERENCES maidan_threads (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX maidan_task_fingerprints_lookup
    ON maidan_task_fingerprints (workspace_id, fingerprint, created_at);
