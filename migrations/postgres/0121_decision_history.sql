-- Review verdicts and land-gate verdicts keep their history. The current
-- tables hold one row per reviewer and one pointer per thread, and a new
-- decision replaces the old; these append every decision and are never
-- updated. Existing decisions are carried over as each thread's first entry.
CREATE TABLE IF NOT EXISTS maidan_thread_review_verdicts (
    id          BIGSERIAL PRIMARY KEY,
    thread_id   UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reviewer_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    decision    TEXT NOT NULL CHECK (decision IN ('approve', 'request_changes')),
    note        TEXT,
    actor_id    UUID,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_review_verdicts_thread
    ON maidan_thread_review_verdicts (thread_id, id);
INSERT INTO maidan_thread_review_verdicts
    (thread_id, reviewer_id, decision, note, actor_id, recorded_at)
SELECT thread_id, reviewer_id, decision, note, actor_id, updated_at
FROM maidan_thread_reviews
ORDER BY updated_at, thread_id, reviewer_id;

CREATE TABLE IF NOT EXISTS maidan_thread_land_gate_verdicts (
    id                BIGSERIAL PRIMARY KEY,
    thread_id         UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    status            TEXT NOT NULL CHECK (status IN ('pass', 'fail')),
    land              TEXT NOT NULL CHECK (land IN ('green', 'amber', 'red')),
    artifact_sha      TEXT,
    recorded_by       UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    recorded_actor_id UUID,
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_land_gate_verdicts_thread
    ON maidan_thread_land_gate_verdicts (thread_id, id);
INSERT INTO maidan_thread_land_gate_verdicts
    (thread_id, status, land, artifact_sha, recorded_by, recorded_actor_id, recorded_at)
SELECT thread_id, status, land, artifact_sha, recorded_by, recorded_actor_id, recorded_at
FROM maidan_thread_land_gate
WHERE status IS NOT NULL
ORDER BY recorded_at, thread_id;
