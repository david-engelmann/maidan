-- Evidence records: what an agent or a person attaches to a thread to show
-- the work happened (a test run, a CI run, a diff, an outside review verdict,
-- a screenshot by URL, or a note). Append-only. A record is read on the
-- thread's evidence timeline but is not pinned at a review hand-off, so it
-- does not move the thread version.
CREATE TABLE maidan_thread_evidence_records (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('test_run', 'ci_run', 'diff', 'review_verdict', 'screenshot', 'note')),
    source TEXT NOT NULL,
    url TEXT,
    sha256 TEXT,
    note TEXT,
    recorded_by UUID NOT NULL REFERENCES maidan_members(id),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (url IS NOT NULL OR sha256 IS NOT NULL OR note IS NOT NULL)
);

CREATE INDEX maidan_thread_evidence_records_thread
    ON maidan_thread_evidence_records (thread_id, recorded_at, id);
