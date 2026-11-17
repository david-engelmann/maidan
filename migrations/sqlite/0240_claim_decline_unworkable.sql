-- A claimant may now decline any thread it cannot work (`release_claim` with
-- `decline: "unworkable"`: its head would not fetch, or it names nothing to
-- work on), not only a review of its own family's change. Without it, a seat
-- that could not fetch a stale review's head was handed the same review on
-- every pass, since the queue hands out its oldest work first. SQLite cannot
-- alter a CHECK, so the table is rebuilt with its rows.
CREATE TABLE maidan_claim_declines_new (
    thread_id   TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id   TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason      TEXT NOT NULL CHECK (reason IN ('own_family', 'unworkable')),
    declined_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (thread_id, member_id)
);
INSERT INTO maidan_claim_declines_new (thread_id, member_id, reason, declined_at)
    SELECT thread_id, member_id, reason, declined_at FROM maidan_claim_declines;
DROP TABLE maidan_claim_declines;
ALTER TABLE maidan_claim_declines_new RENAME TO maidan_claim_declines;
