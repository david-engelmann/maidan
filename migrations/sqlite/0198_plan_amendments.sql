-- How many times each plan was amended (#1465 part 3); see the Postgres twin.
ALTER TABLE maidan_thread_plans ADD COLUMN amendments INTEGER NOT NULL DEFAULT 0;
