-- Claim and release comments; see the Postgres twin (0188).
ALTER TABLE maidan_workspaces ADD COLUMN linear_claim_comments INTEGER NOT NULL DEFAULT 0;

ALTER TABLE maidan_thread_attributes ADD COLUMN linear_issue TEXT;

CREATE TABLE maidan_linear_notices (
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    issue        TEXT NOT NULL,
    outcome      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (thread_id, kind)
);
