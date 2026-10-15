-- Swarm ramp metrics; see the Postgres twin (0174).
CREATE TABLE maidan_change_pulls (
    repo            TEXT NOT NULL,
    pull_number     INTEGER NOT NULL,
    workspace_id    TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id       TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    opened_at       TEXT NOT NULL,
    ready_at        TEXT,
    merged_at       TEXT,
    merged_head_sha TEXT,
    closed_at       TEXT,
    PRIMARY KEY (repo, pull_number)
);
CREATE INDEX idx_change_pulls_workspace ON maidan_change_pulls (workspace_id, opened_at);

CREATE TABLE maidan_change_rounds (
    repo         TEXT NOT NULL,
    pull_number  INTEGER NOT NULL,
    commit_sha   TEXT NOT NULL,
    round        INTEGER NOT NULL,
    base_sha     TEXT NOT NULL,
    base_behind  INTEGER,
    diff_lines   INTEGER NOT NULL,
    delivered_at TEXT NOT NULL,
    PRIMARY KEY (repo, pull_number, commit_sha),
    FOREIGN KEY (repo, pull_number) REFERENCES maidan_change_pulls(repo, pull_number) ON DELETE CASCADE
);
