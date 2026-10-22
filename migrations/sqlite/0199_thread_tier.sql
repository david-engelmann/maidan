-- A change's review tier (WAX-D-079); see the Postgres twin.
ALTER TABLE maidan_thread_attributes ADD COLUMN tier TEXT CHECK (tier IN ('A', 'B'));
