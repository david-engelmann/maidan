-- A workspace's ceiling on delegation-grant lifetime. See the Postgres twin.
CREATE TABLE maidan_delegation_policies (
    workspace_id   TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    max_grant_days INTEGER NOT NULL CHECK (max_grant_days BETWEEN 1 AND 3650),
    updated_at     TEXT NOT NULL
);
