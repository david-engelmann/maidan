-- Cluster 370.5 (Wave 2 #18): a schedule may seed a recipe run instead of a bare
-- thread (SQLite twin of pg 0075). When set, the sweeper instantiates `recipe_id`
-- (parent + DAG children, copy-on-fire) on each firing. NULL = the Cluster-226
-- one-thread behaviour. (SQLite can't add an FK via ALTER; the app enforces the
-- reference, and a deleted recipe is tolerated as a fallback-to-bare-thread.)
ALTER TABLE maidan_task_schedules ADD COLUMN recipe_id TEXT;
