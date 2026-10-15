-- Swarm ramp metrics (maidan_types::change_metrics). The change flow records
-- each agent pull request it opens and each round it delivers to one;
-- mark-ready and a refresh stamp when it was made ready and merged.
CREATE TABLE maidan_change_pulls (
    repo            TEXT NOT NULL,
    pull_number     BIGINT NOT NULL,
    workspace_id    UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id       UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    opened_at       TIMESTAMPTZ NOT NULL,
    ready_at        TIMESTAMPTZ,
    merged_at       TIMESTAMPTZ,
    merged_head_sha TEXT,
    closed_at       TIMESTAMPTZ,
    PRIMARY KEY (repo, pull_number)
);
CREATE INDEX idx_change_pulls_workspace ON maidan_change_pulls (workspace_id, opened_at);

-- One row per delivered commit; a retried delivery that reuses its commit
-- adds no round.
CREATE TABLE maidan_change_rounds (
    repo         TEXT NOT NULL,
    pull_number  BIGINT NOT NULL,
    commit_sha   TEXT NOT NULL,
    round        BIGINT NOT NULL,
    base_sha     TEXT NOT NULL,
    base_behind  BIGINT,
    diff_lines   BIGINT NOT NULL,
    delivered_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (repo, pull_number, commit_sha),
    FOREIGN KEY (repo, pull_number) REFERENCES maidan_change_pulls(repo, pull_number) ON DELETE CASCADE
);
