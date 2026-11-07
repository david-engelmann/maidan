-- A change's review tier; see the Postgres twin.
ALTER TABLE maidan_thread_attributes ADD COLUMN tier TEXT CHECK (tier IN ('A', 'B'));
