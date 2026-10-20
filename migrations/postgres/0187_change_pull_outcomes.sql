-- Outcome metrics for merged agent pull requests (Next 33, #1462): whether
-- the change held. The change sweep reads them from GitHub in an hourly
-- batch per workspace, each pull request at most once a day for 30 days
-- after its merge (`outcome_checked_at`).
--
-- `merge_commit_sha` is the commit the merge made on the base, which a
-- revert names. `followup_fixes` lists the later merged pull requests that
-- say they fix this one. The human-effort proxies (commits not delivered by
-- the change flow, reviews and review comments by accounts that are not
-- bots) are read once, after the merge; NULL until then.
ALTER TABLE maidan_change_pulls
    ADD COLUMN merge_commit_sha TEXT,
    ADD COLUMN reverted_at TIMESTAMPTZ,
    ADD COLUMN revert_ref TEXT,
    ADD COLUMN followup_fixes JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN human_commits BIGINT,
    ADD COLUMN human_reviews BIGINT,
    ADD COLUMN first_human_review_at TIMESTAMPTZ,
    ADD COLUMN outcome_checked_at TIMESTAMPTZ;
