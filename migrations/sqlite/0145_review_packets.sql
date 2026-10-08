-- Open Work Next 3 (evidence-bound approvals), part 2. Each start_review
-- records what the thread put in front of its reviewers: its version, its
-- result and its linked artifacts by content hash, and the root of that
-- manifest. A packet is never updated: a later hand-off writes a new one. It
-- goes when its thread does.
CREATE TABLE maidan_review_packets (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    requested_by TEXT NOT NULL REFERENCES maidan_members(id),
    thread_version INTEGER NOT NULL,
    manifest TEXT NOT NULL,
    evidence_root TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_review_packets_thread ON maidan_review_packets (thread_id, created_at DESC);

CREATE TRIGGER maidan_review_packets_no_update
BEFORE UPDATE ON maidan_review_packets
BEGIN
    SELECT RAISE(ABORT, 'a review packet is immutable: start a new review instead');
END;
