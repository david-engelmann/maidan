-- Cluster 384 (Wave 2 #25 remainder, G-dev-6): Soundcheck gate pointer
-- (SQLite twin of pg 0088). Presence of a row arms the close-gate.
CREATE TABLE IF NOT EXISTS maidan_thread_soundcheck (
    thread_id    TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    status       TEXT CHECK (status IS NULL OR status IN ('pass', 'fail')),
    land         TEXT CHECK (land IS NULL OR land IN ('green', 'amber', 'red')),
    artifact_sha TEXT,
    recorded_by  TEXT REFERENCES maidan_members(id) ON DELETE CASCADE,
    recorded_at  TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    CHECK (
        (status IS NULL AND land IS NULL AND recorded_by IS NULL
         AND recorded_at IS NULL AND artifact_sha IS NULL)
        OR
        (status IS NOT NULL AND land IS NOT NULL AND recorded_by IS NOT NULL
         AND recorded_at IS NOT NULL)
    )
);
