-- Cluster 370.2 (Wave 2 #18): a recipe instantiation — the copy-on-fire record.
-- Firing a recipe (`instantiate_recipe`) builds a parent thread + its DAG children
-- + attaches skills, and writes one row here freezing the recipe bytes
-- (`spec_snapshot`) and the caller's `params` at fire time, so a later edit to the
-- recipe never changes what this run was. `root_thread_id` is the parent thread.
CREATE TABLE IF NOT EXISTS maidan_recipe_runs (
    id             UUID PRIMARY KEY,
    recipe_id      UUID NOT NULL REFERENCES maidan_recipes(id) ON DELETE CASCADE,
    workspace_id   UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    root_thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    params         JSONB NOT NULL,
    spec_snapshot  JSONB NOT NULL,
    created_by     UUID NOT NULL REFERENCES maidan_members(id),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_maidan_recipe_runs_recipe
    ON maidan_recipe_runs(recipe_id, created_at DESC);
