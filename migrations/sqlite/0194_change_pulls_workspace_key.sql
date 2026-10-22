-- Workspace-scoped keys for tracked pull requests; see the Postgres twin
-- (0194). SQLite cannot change a primary key in place, so all three tables are
-- rebuilt. The children are copied out and dropped before the parent, because
-- foreign keys are on and dropping the parent first would cascade-delete them.
CREATE TEMP TABLE tmp_change_pulls AS SELECT * FROM maidan_change_pulls;
CREATE TEMP TABLE tmp_change_rounds AS SELECT * FROM maidan_change_rounds;
CREATE TEMP TABLE tmp_pull_ci AS SELECT * FROM maidan_pull_ci;
DROP TABLE maidan_change_rounds;
DROP TABLE maidan_pull_ci;
DROP TABLE maidan_change_pulls;

CREATE TABLE maidan_change_pulls (
    repo                  TEXT NOT NULL,
    pull_number           INTEGER NOT NULL,
    workspace_id          TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id             TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    opened_at             TEXT NOT NULL,
    ready_at              TEXT,
    merged_at             TEXT,
    merged_head_sha       TEXT,
    closed_at             TEXT,
    noticed_at            TEXT,
    checked_at            TEXT,
    settled_at            TEXT,
    linear_issue          TEXT,
    merge_commit_sha      TEXT,
    reverted_at           TEXT,
    revert_ref            TEXT,
    followup_fixes        TEXT NOT NULL DEFAULT '[]',
    human_commits         INTEGER,
    human_reviews         INTEGER,
    first_human_review_at TEXT,
    outcome_checked_at    TEXT,
    watch_only            INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, repo, pull_number)
);
INSERT INTO maidan_change_pulls (repo, pull_number, workspace_id, thread_id, opened_at,
        ready_at, merged_at, merged_head_sha, closed_at, noticed_at, checked_at, settled_at,
        linear_issue, merge_commit_sha, reverted_at, revert_ref, followup_fixes, human_commits,
        human_reviews, first_human_review_at, outcome_checked_at, watch_only)
    SELECT repo, pull_number, workspace_id, thread_id, opened_at,
        ready_at, merged_at, merged_head_sha, closed_at, noticed_at, checked_at, settled_at,
        linear_issue, merge_commit_sha, reverted_at, revert_ref, followup_fixes, human_commits,
        human_reviews, first_human_review_at, outcome_checked_at, watch_only
    FROM tmp_change_pulls;
CREATE INDEX idx_change_pulls_workspace ON maidan_change_pulls (workspace_id, opened_at);

CREATE TABLE maidan_change_rounds (
    repo          TEXT NOT NULL,
    pull_number   INTEGER NOT NULL,
    commit_sha    TEXT NOT NULL,
    round         INTEGER NOT NULL,
    base_sha      TEXT NOT NULL,
    base_behind   INTEGER,
    diff_lines    INTEGER NOT NULL,
    delivered_at  TEXT NOT NULL,
    paths         TEXT NOT NULL DEFAULT '[]',
    base_branch   TEXT,
    rebase        INTEGER,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    previous_head TEXT,
    PRIMARY KEY (workspace_id, repo, pull_number, commit_sha),
    FOREIGN KEY (workspace_id, repo, pull_number)
        REFERENCES maidan_change_pulls(workspace_id, repo, pull_number) ON DELETE CASCADE
);
-- Rounds written before 0182 may lack a workspace; they take their pull's.
INSERT INTO maidan_change_rounds (repo, pull_number, commit_sha, round, base_sha, base_behind,
        diff_lines, delivered_at, paths, base_branch, rebase, workspace_id, previous_head)
    SELECT r.repo, r.pull_number, r.commit_sha, r.round, r.base_sha, r.base_behind,
        r.diff_lines, r.delivered_at, r.paths, r.base_branch, r.rebase,
        COALESCE(r.workspace_id, p.workspace_id), r.previous_head
    FROM tmp_change_rounds r
    JOIN tmp_change_pulls p ON p.repo = r.repo AND p.pull_number = r.pull_number;

CREATE TABLE maidan_pull_ci (
    repo                    TEXT NOT NULL,
    pull_number             INTEGER NOT NULL,
    workspace_id            TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    head_sha                TEXT NOT NULL,
    status                  TEXT NOT NULL,
    failing_jobs            TEXT NOT NULL DEFAULT '[]',
    log_tail                TEXT,
    updated_at              TEXT NOT NULL,
    merge_state             TEXT,
    unresolved_threads      INTEGER NOT NULL DEFAULT 0,
    unresolved_thread_links TEXT NOT NULL DEFAULT '[]',
    missing_checks          TEXT NOT NULL DEFAULT '[]',
    gate_reasons            TEXT,
    merge_queue             TEXT,
    merge_queue_reason      TEXT,
    head_ahead              INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, repo, pull_number),
    FOREIGN KEY (workspace_id, repo, pull_number)
        REFERENCES maidan_change_pulls(workspace_id, repo, pull_number) ON DELETE CASCADE
);
INSERT INTO maidan_pull_ci (repo, pull_number, workspace_id, head_sha, status, failing_jobs,
        log_tail, updated_at, merge_state, unresolved_threads, unresolved_thread_links,
        missing_checks, gate_reasons, merge_queue, merge_queue_reason, head_ahead)
    SELECT repo, pull_number, workspace_id, head_sha, status, failing_jobs,
        log_tail, updated_at, merge_state, unresolved_threads, unresolved_thread_links,
        missing_checks, gate_reasons, merge_queue, merge_queue_reason, head_ahead
    FROM tmp_pull_ci;

DROP TABLE tmp_change_pulls;
DROP TABLE tmp_change_rounds;
DROP TABLE tmp_pull_ci;
