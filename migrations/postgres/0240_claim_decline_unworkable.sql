-- A claimant may now decline any thread it cannot work (`release_claim` with
-- `decline: "unworkable"`: its head would not fetch, or it names nothing to
-- work on), not only a review of its own family's change. Without it, a seat
-- that could not fetch a stale review's head was handed the same review on
-- every pass, since the queue hands out its oldest work first.
ALTER TABLE maidan_claim_declines DROP CONSTRAINT IF EXISTS maidan_claim_declines_reason_check;
ALTER TABLE maidan_claim_declines ADD CONSTRAINT maidan_claim_declines_reason_check
    CHECK (reason IN ('own_family', 'unworkable'));
