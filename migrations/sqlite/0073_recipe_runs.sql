-- Cluster 370.2 (Wave 2 #18): a recipe instantiation — the copy-on-fire record
-- (SQLite twin of pg 0074). Firing a recipe builds a parent thread + DAG children
-- + skills and writes one row freezing the recipe bytes (`spec_snapshot`) + the
-- caller's `params` at fire time. `root_thread_id` is the parent thread.
CREATE TABLE IF NOT EXISTS maidan_recipe_runs (
    id             TEXT PRIMARY KEY,
    recipe_id      TEXT NOT NULL REFERENCES maidan_recipes(id) ON DELETE CASCADE,
    workspace_id   TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    root_thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    params         TEXT NOT NULL,
    spec_snapshot  TEXT NOT NULL,
    created_by     TEXT NOT NULL REFERENCES maidan_members(id),
    created_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_maidan_recipe_runs_recipe
    ON maidan_recipe_runs(recipe_id, created_at DESC);
