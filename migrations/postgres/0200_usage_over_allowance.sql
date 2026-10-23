-- A workspace's crossing of the instance's free allowance on a meter in a
-- UTC month (`MAIDAN_FREE_ALLOWANCE`): recorded once, with the event and its
-- admins' notifications in the same transaction. Nothing is cut off or billed.
CREATE TABLE maidan_usage_over_allowance (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    meter        TEXT NOT NULL,
    month        TEXT NOT NULL,
    quantity     BIGINT NOT NULL,
    allowance    BIGINT NOT NULL,
    marked_at    TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (workspace_id, meter, month)
);
CREATE INDEX idx_usage_over_allowance_month ON maidan_usage_over_allowance (month);
