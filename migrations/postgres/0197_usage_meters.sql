-- Per-workspace usage meters, one row per workspace, UTC day and meter: the
-- daily rollup the free allowance is checked against. Written in batches by
-- the usage_meters flush loop, adding to the day's quantity.
CREATE TABLE maidan_usage_meters (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    day          DATE NOT NULL,
    meter        TEXT NOT NULL,
    quantity     BIGINT NOT NULL,
    PRIMARY KEY (workspace_id, day, meter)
);
