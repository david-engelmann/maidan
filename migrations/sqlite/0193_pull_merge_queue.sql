-- Merge-queue state of a tracked pull request; see the Postgres twin (0193).
ALTER TABLE maidan_pull_ci ADD COLUMN merge_queue TEXT;
ALTER TABLE maidan_pull_ci ADD COLUMN merge_queue_reason TEXT;
ALTER TABLE maidan_pull_ci ADD COLUMN head_ahead INTEGER NOT NULL DEFAULT 0;
