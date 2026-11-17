-- How long `claim_next` holds a thread back from a member whose plain
-- releases of it showed no progress. See the Postgres twin.
ALTER TABLE maidan_thread_workers ADD COLUMN empty_releases INTEGER NOT NULL DEFAULT 0;
ALTER TABLE maidan_thread_workers ADD COLUMN reclaim_after TEXT;
