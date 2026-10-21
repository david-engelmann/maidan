-- What holds a tracked pull request back; see the Postgres twin (0192).
ALTER TABLE maidan_pull_ci ADD COLUMN merge_state TEXT;
ALTER TABLE maidan_pull_ci ADD COLUMN unresolved_threads INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_pull_ci ADD COLUMN unresolved_thread_links TEXT NOT NULL DEFAULT '[]';
ALTER TABLE maidan_pull_ci ADD COLUMN missing_checks TEXT NOT NULL DEFAULT '[]';
ALTER TABLE maidan_pull_ci ADD COLUMN gate_reasons TEXT;

ALTER TABLE maidan_thread_notices ADD COLUMN reasons TEXT NOT NULL DEFAULT '[]';
