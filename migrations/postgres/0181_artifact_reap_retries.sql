-- Orphaned artifact bytes whose delete failed or overran, waiting for the
-- retention sweep to try again. Before, the failure was only logged and the
-- bytes stayed in the blob store with nothing left to find them.
--
-- Keyed by sha: the bytes are stored once however many workspaces uploaded
-- them, so one row stands for them. `workspace_id` is the workspace whose
-- erase, purge, expiry or refused upload orphaned the sha, kept for the
-- operator. It has no foreign key: a purge deletes that workspace, and its
-- bytes must still go.
CREATE TABLE IF NOT EXISTS maidan_artifact_reap_retries (
    sha256 TEXT PRIMARY KEY,
    workspace_id UUID NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS maidan_artifact_reap_retries_due
    ON maidan_artifact_reap_retries (next_attempt_at);
