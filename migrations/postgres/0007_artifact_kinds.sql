-- Maidan artifact kind constraint, v0007, Postgres dialect.

ALTER TABLE maidan_artifacts
    ADD CONSTRAINT maidan_artifacts_kind_check
    CHECK (kind IN (
        'screenshot',
        'recording',
        'transcript',
        'code_dump',
        'attachment'
    ));
