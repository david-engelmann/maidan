-- An artifact's bytes are shared across workspaces (content-addressed), but
-- what a workspace declared about them is not. The shared row kept the first
-- uploader's `uploaded_by` and `created_at` and the latest uploader's `kind`
-- and `mime_type`, so a second tenant uploading the same bytes read back the
-- first tenant's member id and could overwrite what the first tenant saw.
-- Each workspace's access ref now carries that workspace's own metadata.
ALTER TABLE maidan_artifact_refs
    ADD COLUMN kind TEXT,
    ADD COLUMN mime_type TEXT,
    ADD COLUMN uploaded_by UUID;

-- Backfill from the shared row. `uploaded_by` only where the uploader belongs
-- to the ref's workspace: anywhere else it names another tenant's member.
UPDATE maidan_artifact_refs r
SET kind = a.kind,
    mime_type = a.mime_type,
    uploaded_by = CASE WHEN m.workspace_id = r.workspace_id THEN a.uploaded_by END
FROM maidan_artifacts a
LEFT JOIN maidan_members m ON m.id = a.uploaded_by
WHERE a.sha256 = r.sha256;
