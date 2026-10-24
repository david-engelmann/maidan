-- The same-issue duplicate hold (C8 follow-up); see the Postgres twin.
ALTER TABLE maidan_task_fingerprints ADD COLUMN cleared_at TEXT;

CREATE INDEX maidan_thread_attributes_linear_issue
    ON maidan_thread_attributes (linear_issue)
    WHERE linear_issue IS NOT NULL;
