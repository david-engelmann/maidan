-- A workspace's default engine; see the Postgres twin (0190).
ALTER TABLE maidan_workspaces
    ADD COLUMN default_engine TEXT CHECK (default_engine IN ('pi', 'session', 'human'));
