-- A workspace's own retention for its messages, events and finished
-- deliveries, in days. NULL keeps that kind as long as the instance does; no
-- row means nothing is set. A table rather than columns on maidan_workspaces:
-- the sweeper reads only the workspaces that set one, and no workspace read or
-- export has to carry it. Bounded by the instance's MAIDAN_RETENTION_*_DAYS at
-- write; a legal hold outranks it at prune. Set with token:admin.
CREATE TABLE maidan_retention_policies (
    workspace_id    UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    messages_days   BIGINT CHECK (messages_days BETWEEN 1 AND 3650),
    events_days     BIGINT CHECK (events_days BETWEEN 1 AND 3650),
    deliveries_days BIGINT CHECK (deliveries_days BETWEEN 1 AND 3650),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
