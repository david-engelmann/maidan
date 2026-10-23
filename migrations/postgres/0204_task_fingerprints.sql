-- Task fingerprints (Context Economics C8): a hash of a task's brief and
-- inputs, stored when the task is created, with the earlier open task in
-- the same workspace that had the same fingerprint at that moment. The
-- workspace is copied in so the duplicate lookup is one index probe.
CREATE TABLE maidan_task_fingerprints (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads (id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL CHECK (fingerprint ~ '^[0-9a-f]{64}$'),
    duplicate_of UUID REFERENCES maidan_threads (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX maidan_task_fingerprints_lookup
    ON maidan_task_fingerprints (workspace_id, fingerprint, created_at);
