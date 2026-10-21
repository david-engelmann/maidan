-- What holds a tracked pull request back (crates/maidan-server/src/ci_watch.rs),
-- from the same per-tick read as its checks: GitHub's merge state, the
-- unresolved review threads, and required checks that never reported.
-- `gate_reasons` is NULL until a read could say, so the first read that
-- finds nothing in the way is not news.
ALTER TABLE maidan_pull_ci
    ADD COLUMN merge_state TEXT,
    ADD COLUMN unresolved_threads BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN unresolved_thread_links JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN missing_checks JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN gate_reasons JSONB;

-- A gate_blocked notice says why.
ALTER TABLE maidan_thread_notices ADD COLUMN reasons JSONB NOT NULL DEFAULT '[]'::jsonb;
