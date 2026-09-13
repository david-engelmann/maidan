-- Cluster 384 (Wave 2 #25 remainder, G-dev-6): Soundcheck gate pointer.
-- Presence of a row arms the close-gate. Pointer columns are NULL until
-- Soundcheck records a {kind:"soundcheck", status:pass|fail, artifact_sha?}
-- plus the green/amber/red land color. The room holds the pointer;
-- Soundcheck owns /test. Not a CI product.
CREATE TABLE IF NOT EXISTS maidan_thread_soundcheck (
    thread_id    UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    status       TEXT CHECK (status IS NULL OR status IN ('pass', 'fail')),
    land         TEXT CHECK (land IS NULL OR land IN ('green', 'amber', 'red')),
    artifact_sha TEXT,
    recorded_by  UUID REFERENCES maidan_members(id) ON DELETE CASCADE,
    recorded_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (status IS NULL AND land IS NULL AND recorded_by IS NULL
         AND recorded_at IS NULL AND artifact_sha IS NULL)
        OR
        (status IS NOT NULL AND land IS NOT NULL AND recorded_by IS NOT NULL
         AND recorded_at IS NOT NULL)
    )
);
