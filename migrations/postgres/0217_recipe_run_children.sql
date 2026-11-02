-- #1729: the children a recipe run created. The warm hold in `claim_next`
-- groups siblings by run id from this table, so only a run's own children are
-- held while the first warms the prompt cache; a sub-thread added to the parent
-- by hand has no row here and is never held. Rows go with the run or the thread.
CREATE TABLE IF NOT EXISTS maidan_recipe_run_children (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    run_id    UUID NOT NULL REFERENCES maidan_recipe_runs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_maidan_recipe_run_children_run
    ON maidan_recipe_run_children(run_id);
