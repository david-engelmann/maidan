-- Remote workspace to poll on the peer's base_url (local workspace_id is ingest target).

ALTER TABLE maidan_peers
    ADD COLUMN remote_workspace_id TEXT;

UPDATE maidan_peers
SET remote_workspace_id = workspace_id
WHERE remote_workspace_id IS NULL;

-- SQLite cannot add NOT NULL in one step after backfill; enforce via application on insert.
