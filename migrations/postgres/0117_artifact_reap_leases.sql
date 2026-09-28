-- A reap's claim on an orphaned artifact's bytes while it deletes them. The
-- reap checks that no artifact row holds the sha and writes this lease in one
-- short transaction, then deletes the bytes outside any transaction; every
-- artifact-row write for the sha waits while the lease is live. Before, the
-- reap held its transaction (and, on SQLite, the database write lock) for the
-- whole blob delete. A lease a crashed reaper left lapses at `expires_at`.
CREATE TABLE IF NOT EXISTS maidan_artifact_reaps (
    sha256 TEXT PRIMARY KEY,
    token UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
