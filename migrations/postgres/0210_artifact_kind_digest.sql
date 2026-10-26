-- Next 38 C10, summarize once: widen the artifact-kind CHECK to allow thread
-- digests, a summary written once and read by every agent on the thread.
ALTER TABLE maidan_artifacts DROP CONSTRAINT maidan_artifacts_kind_check;
ALTER TABLE maidan_artifacts
    ADD CONSTRAINT maidan_artifacts_kind_check
    CHECK (kind IN (
        'screenshot',
        'recording',
        'transcript',
        'code_dump',
        'attachment',
        'context_snapshot',
        'digest'
    ));
