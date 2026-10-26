-- How a workspace came to be; see the Postgres twin (0209).
ALTER TABLE maidan_workspaces
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'operator'
        CHECK (origin IN ('operator', 'signup', 'open'));
