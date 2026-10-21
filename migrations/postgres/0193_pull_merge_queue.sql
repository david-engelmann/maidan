-- Where a tracked pull request stands with its base branch's merge queue
-- (crates/maidan-server/src/ci_watch.rs), from the same per-tick read:
-- ready_to_merge, queued or dequeued, with GitHub's reason for a dequeue.
-- Landed is not stored: a merge recorded on the pull request says it.
--
-- `head_ahead` says the recorded head had commits its base lacked, at some
-- read while it was the head. A pull request closed without merging counts
-- as landed only when its head is on the base *and* was ahead of it: a
-- branch reset to its base and closed lands nothing.
ALTER TABLE maidan_pull_ci
    ADD COLUMN merge_queue TEXT,
    ADD COLUMN merge_queue_reason TEXT,
    ADD COLUMN head_ahead BOOLEAN NOT NULL DEFAULT false;
