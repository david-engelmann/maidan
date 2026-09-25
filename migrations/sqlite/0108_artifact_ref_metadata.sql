-- Per-workspace artifact metadata; see the Postgres twin (0109).
ALTER TABLE maidan_artifact_refs ADD COLUMN kind TEXT;
ALTER TABLE maidan_artifact_refs ADD COLUMN mime_type TEXT;
ALTER TABLE maidan_artifact_refs ADD COLUMN uploaded_by TEXT;

UPDATE maidan_artifact_refs
SET kind = (SELECT a.kind FROM maidan_artifacts a WHERE a.sha256 = maidan_artifact_refs.sha256),
    mime_type = (SELECT a.mime_type FROM maidan_artifacts a WHERE a.sha256 = maidan_artifact_refs.sha256),
    uploaded_by = (
        SELECT a.uploaded_by FROM maidan_artifacts a
        JOIN maidan_members m ON m.id = a.uploaded_by
        WHERE a.sha256 = maidan_artifact_refs.sha256
          AND m.workspace_id = maidan_artifact_refs.workspace_id
    );
