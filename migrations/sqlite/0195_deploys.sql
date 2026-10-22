-- Deploys a deploy hook reported; see the Postgres twin (0195).
CREATE TABLE maidan_deploys (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    pull_number  INTEGER NOT NULL,
    sha          TEXT NOT NULL,
    environment  TEXT NOT NULL,
    status       TEXT NOT NULL,
    url          TEXT,
    received_at  TEXT NOT NULL,
    UNIQUE (workspace_id, repo, pull_number, sha, environment, status)
);

CREATE INDEX maidan_deploys_thread ON maidan_deploys (thread_id, received_at);

ALTER TABLE maidan_thread_notices ADD COLUMN environment TEXT;
ALTER TABLE maidan_thread_notices ADD COLUMN deploy_url TEXT;
