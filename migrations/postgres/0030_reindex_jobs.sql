-- Persisted embedding reindex jobs (Cluster 104.0.3).
-- Replaces the in-memory ReindexJobRegistry so a job's status is visible on any
-- replica and survives restart. `processed`/`failed` are counts; a NULL
-- `finished_at` means the job is still running.
CREATE TABLE maidan_reindex_jobs (
    job_id          UUID PRIMARY KEY,
    status          TEXT NOT NULL,
    workspace_id    UUID REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    embedding_model TEXT NOT NULL,
    processed       BIGINT,
    failed          BIGINT,
    error           TEXT,
    started_at      TIMESTAMPTZ NOT NULL,
    finished_at     TIMESTAMPTZ
);

CREATE INDEX idx_reindex_jobs_started_at ON maidan_reindex_jobs (started_at DESC);
