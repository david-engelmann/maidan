-- A workspace's unique thread attributes; see the Postgres twin (0223).
ALTER TABLE maidan_workspaces
    ADD COLUMN unique_thread_attributes TEXT NOT NULL DEFAULT '[]';
