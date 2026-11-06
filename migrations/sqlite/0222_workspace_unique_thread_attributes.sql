-- A workspace's unique thread attribute key sets, and the task attribute
-- `repo`; see the Postgres twin (0223).
ALTER TABLE maidan_workspaces
    ADD COLUMN unique_thread_attributes TEXT NOT NULL DEFAULT '[]';

ALTER TABLE maidan_thread_attributes ADD COLUMN repo TEXT;
