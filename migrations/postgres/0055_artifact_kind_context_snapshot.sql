-- Cluster 329: widen the artifact-kind CHECK to allow context snapshots
-- (a frozen, content-addressed context pack). Postgres dialect.
ALTER TABLE maidan_artifacts DROP CONSTRAINT maidan_artifacts_kind_check;
ALTER TABLE maidan_artifacts
    ADD CONSTRAINT maidan_artifacts_kind_check
    CHECK (kind IN (
        'screenshot',
        'recording',
        'transcript',
        'code_dump',
        'attachment',
        'context_snapshot'
    ));
