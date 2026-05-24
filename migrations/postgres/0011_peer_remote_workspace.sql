-- Remote workspace to poll on the peer's base_url (local workspace_id is ingest target).

ALTER TABLE maidan_peers
    ADD COLUMN remote_workspace_id UUID;

UPDATE maidan_peers
SET remote_workspace_id = workspace_id
WHERE remote_workspace_id IS NULL;

ALTER TABLE maidan_peers
    ALTER COLUMN remote_workspace_id SET NOT NULL;
