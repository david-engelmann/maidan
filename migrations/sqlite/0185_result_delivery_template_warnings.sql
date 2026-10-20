-- The pull-request template check (Next 33, #1463): what a delivered
-- change's pull request lacks against the producer's template, as
-- comma-separated codes (`missing_testing`, `missing_issue_key`). NULL when
-- nothing is missing or the row is not a change. A flag, never a failure:
-- `last_error` keeps meaning "this did not land".
ALTER TABLE maidan_result_deliveries ADD COLUMN template_warnings TEXT;
