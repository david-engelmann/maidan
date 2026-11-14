-- A memory block's revision: 1 at creation, plus one per rewrite.
-- `set_memory_block_value` now names the revision it read, and the store
-- refuses the write if the block has moved past it. Several agents converge on
-- one block by label and read, change and write it back. Before this, two such
-- writers lost one of their updates without either knowing.
ALTER TABLE maidan_memory_blocks ADD COLUMN IF NOT EXISTS revision BIGINT NOT NULL DEFAULT 1;
