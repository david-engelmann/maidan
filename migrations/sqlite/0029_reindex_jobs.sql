-- Persisted embedding reindex jobs (Cluster 104.0.3). SQLite mirror.
CREATE TABLE maidan_reindex_jobs (
    job_id          TEXT PRIMARY KEY,
    status          TEXT NOT NULL,
    workspace_id    TEXT REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    embedding_model TEXT NOT NULL,
    processed       INTEGER,
    failed          INTEGER,
    error           TEXT,
    started_at      TEXT NOT NULL,
    finished_at     TEXT
);

CREATE INDEX idx_reindex_jobs_started_at ON maidan_reindex_jobs (started_at DESC);
