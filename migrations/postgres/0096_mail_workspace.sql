-- Cluster 398.3: scope the mail outbox to a workspace.
--
-- `maidan_mail_outbox` had no workspace column, and `GET /operator/mail/dead`
-- was gated on `token:admin` — a **per-workspace** capability minted through
-- `POST /workspaces/:wid/members/:mid/tokens`. The query was global, so any
-- workspace admin could read every other tenant's outbound email: recipient
-- address, subject and body.
--
-- Going forward the workspace is known at enqueue time (the notification router
-- has it), so new rows carry it and the operator routes scope to the caller's
-- workspace.
--
-- Nullable on purpose. Rows written before this migration have no workspace to
-- recover — the table never stored one — and mail sent outside a workspace
-- context never will. Those rows are visible only to a caller holding the new
-- `operator:global` capability, which is the honest reading: "this row cannot be
-- attributed to a tenant, so only an instance operator may see it."
ALTER TABLE maidan_mail_outbox
    ADD COLUMN workspace_id UUID REFERENCES maidan_workspaces (id) ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS idx_mail_outbox_workspace_dead
    ON maidan_mail_outbox (workspace_id, updated_at DESC)
    WHERE status = 'dead';
