-- A memory block's revision: 1 at creation, plus one per rewrite. See the
-- Postgres twin (0238) for why.
ALTER TABLE maidan_memory_blocks ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
