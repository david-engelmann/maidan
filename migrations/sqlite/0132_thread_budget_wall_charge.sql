-- Seconds worked by claims on a thread whose lease lapsed. See the Postgres
-- twin.
ALTER TABLE maidan_thread_budgets ADD COLUMN used_wall_secs INTEGER NOT NULL DEFAULT 0;
