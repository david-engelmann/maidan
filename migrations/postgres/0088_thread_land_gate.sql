-- Cluster 385 (Wave 2 #25 remainder; Cluster 389 renamed the public
-- surface to land_gate). Presence of a row arms the close-gate. Pointer
-- columns are NULL until a gate-skilled member records
-- {kind:"land_gate", status:pass|fail, artifact_sha?} plus the
-- green/amber/red land color. The room holds the pointer; an external
-- verifier records pass/fail. Not a CI product.
CREATE TABLE IF NOT EXISTS maidan_thread_land_gate (
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
