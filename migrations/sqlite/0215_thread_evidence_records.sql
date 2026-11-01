-- SQLite mirror of postgres 0216. Evidence records: what an agent or a person
-- attaches to a thread to show the work happened. Append-only, and not part
-- of the thread version.
CREATE TABLE maidan_thread_evidence_records (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('test_run', 'ci_run', 'diff', 'review_verdict', 'screenshot', 'note')),
    source TEXT NOT NULL,
    url TEXT,
    sha256 TEXT,
    note TEXT,
    recorded_by TEXT NOT NULL REFERENCES maidan_members(id),
    recorded_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (url IS NOT NULL OR sha256 IS NOT NULL OR note IS NOT NULL)
);

CREATE INDEX maidan_thread_evidence_records_thread
    ON maidan_thread_evidence_records (thread_id, recorded_at, id);
