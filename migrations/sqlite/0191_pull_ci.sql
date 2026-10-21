-- CI on tracked pull requests; see the Postgres twin (0191).
ALTER TABLE maidan_change_pulls ADD COLUMN watch_only INTEGER NOT NULL DEFAULT 0;

CREATE TABLE maidan_pull_ci (
    repo         TEXT NOT NULL,
    pull_number  INTEGER NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    head_sha     TEXT NOT NULL,
    status       TEXT NOT NULL,
    failing_jobs TEXT NOT NULL DEFAULT '[]',
    log_tail     TEXT,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (repo, pull_number),
    FOREIGN KEY (repo, pull_number) REFERENCES maidan_change_pulls(repo, pull_number) ON DELETE CASCADE
);

ALTER TABLE maidan_thread_notices ADD COLUMN head_sha TEXT;
ALTER TABLE maidan_thread_notices ADD COLUMN status TEXT;
ALTER TABLE maidan_thread_notices ADD COLUMN failing_jobs TEXT NOT NULL DEFAULT '[]';
