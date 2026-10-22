-- Deploys a deploy hook reported (POST /integrations/deploys/events).
--
-- One row per tracked pull request whose merge commit the hook named, per
-- environment and outcome: a retried hook call finds its row and says
-- nothing twice. There is no foreign key to maidan_change_pulls, whose key
-- 0194 changes; the thread and workspace hold the row's lifetime instead.
CREATE TABLE maidan_deploys (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id    UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    repo         TEXT NOT NULL,
    pull_number  BIGINT NOT NULL,
    sha          TEXT NOT NULL,
    environment  TEXT NOT NULL,
    status       TEXT NOT NULL,
    url          TEXT,
    received_at  TIMESTAMPTZ NOT NULL,
    UNIQUE (workspace_id, repo, pull_number, sha, environment, status)
);

CREATE INDEX maidan_deploys_thread ON maidan_deploys (thread_id, received_at);

-- A deploy notice names where the commit went and the deploy's page.
ALTER TABLE maidan_thread_notices
    ADD COLUMN environment TEXT,
    ADD COLUMN deploy_url TEXT;
