-- A claimant that releases a review because the change was written by its own
-- model family says so with the release (`release_claim` with
-- `decline: "own_family"`), and claim_next stops handing that review to that
-- member. The usage-evidence guard (0202) cannot refuse a seat that has
-- reported no usage yet, so without this the seat was handed the same oldest
-- review on every pass. One row per (thread, member): other members are still
-- handed the review, and a later review round is a new thread. Written only by
-- the holder's fenced release, so the member and the thread share a workspace.
CREATE TABLE IF NOT EXISTS maidan_claim_declines (
    thread_id   TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id   TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    reason      TEXT NOT NULL CHECK (reason IN ('own_family')),
    declined_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (thread_id, member_id)
);
