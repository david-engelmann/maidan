-- Every audit row belongs to a workspace, or says it belongs to none.
--
-- Until now a row carried no workspace. The workspace audit view guessed one
-- from the actor's membership or a `workspace` target, so a row with no actor
-- (an app-token mint, a SCIM change, a result-delivery attempt) or with a
-- different target (a token, a legal hold) was in no workspace's view, and
-- retention could not tell whose a row was, so any legal hold froze audit
-- pruning for the whole instance.
--
-- Writers now stamp the workspace. Existing rows are backfilled from what they
-- reference, first match wins:
--   1. a `workspace` target: its id;
--   2. `metadata.workspace_id`, which most workspace-scoped writers recorded;
--   3. the target's own row (token, member, channel, thread, message, share
--      ticket, grant, egress target, app installation, result delivery,
--      reindex job, legal hold);
--   4. the actor's workspace, then the subject's (not for a reindex, where
--      no workspace means the whole instance).
-- A row none of these resolves (its members and target erased, or an
-- instance-wide operation) stays NULL: instance-level, shown only in the
-- operator audit, and pruned under instance policy, never kept by a hold.
--
-- No foreign key, for the reason `actor_id` has none (0108): an audit row
-- keeps naming what it recorded after the workspace is erased.
ALTER TABLE maidan_audit ADD COLUMN workspace_id UUID;

UPDATE maidan_audit a
SET workspace_id = COALESCE(
    CASE WHEN a.target_kind = 'workspace' THEN a.target_id END,
    CASE
        WHEN jsonb_typeof(a.metadata) = 'object'
         AND a.metadata->>'workspace_id'
             ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
        THEN (a.metadata->>'workspace_id')::uuid
    END,
    CASE a.target_kind
        WHEN 'api_token' THEN
            (SELECT t.workspace_id FROM maidan_api_tokens t WHERE t.id = a.target_id)
        WHEN 'member' THEN
            (SELECT m.workspace_id FROM maidan_members m WHERE m.id = a.target_id)
        WHEN 'channel' THEN
            (SELECT c.workspace_id FROM maidan_channels c WHERE c.id = a.target_id)
        WHEN 'thread' THEN
            (SELECT c.workspace_id FROM maidan_threads t
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE t.id = a.target_id)
        WHEN 'message' THEN
            (SELECT c.workspace_id FROM maidan_messages m
             JOIN maidan_threads t ON t.id = m.thread_id
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE m.id = a.target_id)
        WHEN 'share_ticket' THEN
            (SELECT s.workspace_id FROM maidan_share_tickets s WHERE s.id = a.target_id)
        WHEN 'delegation_grant' THEN
            (SELECT g.workspace_id FROM maidan_delegation_grants g WHERE g.id = a.target_id)
        WHEN 'egress_target' THEN
            (SELECT e.workspace_id FROM maidan_egress_targets e WHERE e.id = a.target_id)
        WHEN 'app_installation' THEN
            (SELECT i.workspace_id FROM maidan_app_installations i WHERE i.id = a.target_id)
        WHEN 'result_delivery' THEN
            (SELECT c.workspace_id FROM maidan_result_deliveries r
             JOIN maidan_threads t ON t.id = r.thread_id
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE r.id = a.target_id)
        WHEN 'reindex_job' THEN
            (SELECT j.workspace_id FROM maidan_reindex_jobs j WHERE j.job_id = a.target_id)
        WHEN 'legal_hold' THEN
            (SELECT h.workspace_id FROM maidan_legal_holds h WHERE h.id = a.target_id)
    END,
    -- An instance-wide reindex names no workspace; its operator's is not it.
    CASE WHEN a.action <> 'embeddings.reindex' THEN
        COALESCE(
            (SELECT m.workspace_id FROM maidan_members m WHERE m.id = a.actor_id),
            (SELECT m.workspace_id FROM maidan_members m WHERE m.id = a.subject_id)
        )
    END
);

CREATE INDEX idx_audit_workspace ON maidan_audit (workspace_id, occurred_at DESC);
