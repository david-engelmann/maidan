-- Review verdicts and land-gate verdicts keep their history. See the Postgres
-- twin.
CREATE TABLE IF NOT EXISTS maidan_thread_review_verdicts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id   TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reviewer_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    decision    TEXT NOT NULL CHECK (decision IN ('approve', 'request_changes')),
    note        TEXT,
    actor_id    TEXT,
    recorded_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_review_verdicts_thread
    ON maidan_thread_review_verdicts (thread_id, id);
INSERT INTO maidan_thread_review_verdicts
    (thread_id, reviewer_id, decision, note, actor_id, recorded_at)
SELECT thread_id, reviewer_id, decision, note, actor_id, updated_at
FROM maidan_thread_reviews
ORDER BY updated_at, thread_id, reviewer_id;

CREATE TABLE IF NOT EXISTS maidan_thread_land_gate_verdicts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id         TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    status            TEXT NOT NULL CHECK (status IN ('pass', 'fail')),
    land              TEXT NOT NULL CHECK (land IN ('green', 'amber', 'red')),
    artifact_sha      TEXT,
    recorded_by       TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    recorded_actor_id TEXT,
    recorded_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_land_gate_verdicts_thread
    ON maidan_thread_land_gate_verdicts (thread_id, id);
INSERT INTO maidan_thread_land_gate_verdicts
    (thread_id, status, land, artifact_sha, recorded_by, recorded_actor_id, recorded_at)
SELECT thread_id, status, land, artifact_sha, recorded_by, recorded_actor_id, recorded_at
FROM maidan_thread_land_gate
WHERE status IS NOT NULL
ORDER BY recorded_at, thread_id;
