-- The Linear issue an agent pull request is for (Next 33, #1462): from the
-- change result's `linear_issue`, else its Linear `deliver_to` target. The
-- outcome metrics match a later fix to it by this key as well as by the
-- pull request's number. NULL when no round named one.
ALTER TABLE maidan_change_pulls ADD COLUMN linear_issue TEXT;
