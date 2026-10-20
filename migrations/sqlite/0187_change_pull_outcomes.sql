-- Outcome metrics for merged agent pull requests; see the Postgres twin
-- (0187).
ALTER TABLE maidan_change_pulls ADD COLUMN merge_commit_sha TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN reverted_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN revert_ref TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN followup_fixes TEXT NOT NULL DEFAULT '[]';
ALTER TABLE maidan_change_pulls ADD COLUMN human_commits INTEGER;
ALTER TABLE maidan_change_pulls ADD COLUMN human_reviews INTEGER;
ALTER TABLE maidan_change_pulls ADD COLUMN first_human_review_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN outcome_checked_at TEXT;
