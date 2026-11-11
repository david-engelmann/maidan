-- Where a bridged message came from, set by the server (RED-007), and the
-- ingress key that makes a bridge's redelivery post nothing (RED-002).
--
-- `origin`: the bridge a message came in through (`github`, `slack`), or
--   NULL for a message a member posted. Only bridge ingress, and the server's
--   own sentences about a linked pull request, write it; no post or edit
--   names it, so a member can neither forge nor strip it. Egress loop
--   prevention reads it instead of the caller-writable `metadata`.
-- `origin_ref`: the bridge's id for what it delivered (a GitHub comment id,
--   a Slack `event_id`); NULL for the server's own sentences.
ALTER TABLE maidan_messages ADD COLUMN origin TEXT
    CHECK (origin IN ('github', 'slack'));
ALTER TABLE maidan_messages ADD COLUMN origin_ref TEXT;

-- One row per external id a bridge has delivered into a workspace, written in
-- the same transaction as the message it posts. A redelivery (a GitHub
-- redeliver, a replayed capture, Slack's retry after 3 s) finds the row and
-- posts nothing. GitHub egress also writes a row for each comment it posts, so
-- the webhook echo of our own comment is taken as already delivered. The row
-- outlives the message: a tombstone or purge must not reopen the key.
CREATE TABLE maidan_bridge_receipts (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    origin TEXT NOT NULL CHECK (origin IN ('github', 'slack')),
    external_id TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, origin, external_id)
);
