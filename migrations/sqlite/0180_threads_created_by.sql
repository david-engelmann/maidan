-- The member who created a thread; see the Postgres twin (0180).
ALTER TABLE maidan_threads
    ADD COLUMN created_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;
