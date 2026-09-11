-- Cluster 370 (Wave 2 #18, G8/W5/G-dev-9): a recipe / thread-type. A recipe is a
-- reusable blueprint (the Goose-recipe *shape* — params, a definition of done, a
-- retry policy, and inline child sub-tasks with a DAG). Instantiating one creates
-- a parent thread + its DAG children + attaches required skills, freezing the
-- recipe bytes into a run snapshot (Cluster 370.2). The spec is stored as one
-- JSON blob (validated against `RecipeSpec` in maidan-types); only the identity +
-- target channel are first-class columns. NOT a recipe VM — a blueprint, not an
-- execution engine.
CREATE TABLE IF NOT EXISTS maidan_recipes (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id   UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    spec         JSONB NOT NULL,
    created_by   UUID NOT NULL REFERENCES maidan_members(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_maidan_recipes_workspace
    ON maidan_recipes(workspace_id, created_at DESC);
