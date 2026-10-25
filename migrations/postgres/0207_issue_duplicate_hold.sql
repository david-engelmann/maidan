-- A change thread whose Linear issue already has another change thread in
-- review is withheld from claim-next as a likely duplicate (C8 follow-up).
-- Clearing the thread's duplicate link records `cleared_at`, which also lifts
-- that hold. The partial index serves the same-issue lookup.
ALTER TABLE maidan_task_fingerprints ADD COLUMN cleared_at TIMESTAMPTZ;

CREATE INDEX maidan_thread_attributes_linear_issue
    ON maidan_thread_attributes (linear_issue)
    WHERE linear_issue IS NOT NULL;
