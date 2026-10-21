-- How many times each plan was amended (amend_plan, #1465 part 3). An
-- amendment past the third is refused, so a planner settles its plan rather
-- than rewriting it under its workers.
ALTER TABLE maidan_thread_plans ADD COLUMN amendments INTEGER NOT NULL DEFAULT 0;
