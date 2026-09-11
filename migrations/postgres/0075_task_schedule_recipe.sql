-- Cluster 370.5 (Wave 2 #18): a schedule may seed a recipe run instead of a bare
-- thread. When set, the sweeper instantiates `recipe_id` (parent + DAG children,
-- copy-on-fire) on each firing instead of creating a single titled thread. NULL =
-- the Cluster-226 behaviour (one thread). ON DELETE SET NULL so deleting a recipe
-- leaves the schedule intact (it falls back to a bare thread).
ALTER TABLE maidan_task_schedules
    ADD COLUMN IF NOT EXISTS recipe_id UUID REFERENCES maidan_recipes(id) ON DELETE SET NULL;
