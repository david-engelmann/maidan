-- Every audit row belongs to a workspace, or says it belongs to none; see the
-- Postgres twin for why and for the backfill order, which this follows. Ids
-- are stored as 16-byte blobs, so a `metadata.workspace_id` string is decoded
-- with `unhex` before it is compared.
ALTER TABLE maidan_audit ADD COLUMN workspace_id TEXT;

UPDATE maidan_audit AS a
SET workspace_id = COALESCE(
    CASE WHEN a.target_kind = 'workspace' THEN a.target_id END,
    CASE
        WHEN json_valid(a.metadata)
         AND json_type(a.metadata, '$.workspace_id') = 'text'
         AND length(json_extract(a.metadata, '$.workspace_id')) = 36
        THEN unhex(replace(json_extract(a.metadata, '$.workspace_id'), '-', ''))
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
