-- A claimant that cannot work (its model provider has no balance, say) took a
-- thread, released it without a word or a token of usage, and was handed the
-- same thread back at once: the queue hands out its oldest work first and
-- breaks ties toward a thread's last holder. One thread went round that loop
-- 113 times in a day.
--
-- `empty_releases` counts the plain releases in a row in which this member
-- reported no usage, posted nothing and stored no result. `reclaim_after` is
-- when `claim_next` may hand the thread back to this member: the app computes
-- it at release, doubling with each count, so the claim query only compares
-- two times. A release that showed progress resets both. Every other member
-- may claim the thread meanwhile. Unlike a decline, the hold expires: a plain
-- release may be transient.
ALTER TABLE maidan_thread_workers
    ADD COLUMN IF NOT EXISTS empty_releases INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_workers ADD COLUMN IF NOT EXISTS reclaim_after TIMESTAMPTZ;
