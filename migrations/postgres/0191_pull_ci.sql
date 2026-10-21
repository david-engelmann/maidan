-- CI on tracked pull requests (crates/maidan-server/src/ci_watch.rs).
--
-- `watch_only` marks a pull request tracked only to read its checks: one
-- someone asked Maidan to watch, on a thread made for it. It is not an agent
-- task, so the swarm metrics and the outcome checks leave it out.
ALTER TABLE maidan_change_pulls ADD COLUMN watch_only BOOLEAN NOT NULL DEFAULT false;

-- Where the checks on a tracked pull request's head stand, as the change
-- sweep last read them: one row per pull request, rewritten each read.
-- `log_tail` is the last 4 KB of the first failing job's log, read once when
-- a head first fails.
CREATE TABLE maidan_pull_ci (
    repo         TEXT NOT NULL,
    pull_number  BIGINT NOT NULL,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    head_sha     TEXT NOT NULL,
    status       TEXT NOT NULL,
    failing_jobs JSONB NOT NULL DEFAULT '[]'::jsonb,
    log_tail     TEXT,
    updated_at   TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (repo, pull_number),
    FOREIGN KEY (repo, pull_number) REFERENCES maidan_change_pulls(repo, pull_number) ON DELETE CASCADE
);

-- A CI notice names the head it is about, and how the checks stood.
ALTER TABLE maidan_thread_notices
    ADD COLUMN head_sha TEXT,
    ADD COLUMN status TEXT,
    ADD COLUMN failing_jobs JSONB NOT NULL DEFAULT '[]'::jsonb;
