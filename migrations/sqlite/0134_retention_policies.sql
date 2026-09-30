-- A workspace's own retention, in days. See the Postgres twin.
CREATE TABLE maidan_retention_policies (
    workspace_id    TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    messages_days   INTEGER CHECK (messages_days BETWEEN 1 AND 3650),
    events_days     INTEGER CHECK (events_days BETWEEN 1 AND 3650),
    deliveries_days INTEGER CHECK (deliveries_days BETWEEN 1 AND 3650),
    updated_at      TEXT NOT NULL
);
