-- Claim and release comments on a task's Linear issue (Next 33, the pilot's
-- entry condition 6). A per-workspace switch, off by default: until an
-- operator turns it on, nothing is said on Linear.
ALTER TABLE maidan_workspaces
    ADD COLUMN linear_claim_comments BOOLEAN NOT NULL DEFAULT FALSE;

-- The Linear issue a task is for, a task attribute set at intake.
ALTER TABLE maidan_thread_attributes ADD COLUMN linear_issue TEXT;

-- One row per notice a task's issue was given (`claim`, `release`), so each
-- is said once, even after its outbox row is gone. `outcome` is `queued`,
-- or why it was not (`skipped: …`).
CREATE TABLE maidan_linear_notices (
    thread_id    UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    issue        TEXT NOT NULL,
    outcome      TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (thread_id, kind)
);
