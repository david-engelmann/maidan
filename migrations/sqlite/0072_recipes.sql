-- Cluster 370 (Wave 2 #18, G8/W5/G-dev-9): a recipe / thread-type (SQLite twin of
-- pg 0073). A reusable blueprint (params, definition of done, retry, inline DAG
-- children); instantiating it (Cluster 370.2) builds a parent thread + DAG
-- children + skills and freezes the recipe bytes into a run snapshot. The spec is
-- one JSON blob (validated against `RecipeSpec`); identity + target channel are
-- the only first-class columns. NOT a recipe VM.
CREATE TABLE IF NOT EXISTS maidan_recipes (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id   TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    spec         TEXT NOT NULL,
    created_by   TEXT NOT NULL REFERENCES maidan_members(id),
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_maidan_recipes_workspace
    ON maidan_recipes(workspace_id, created_at DESC);
