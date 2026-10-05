-- Open Work Next 4 (P0-3): a blocked agent reaches a human.
-- `set_thread_block` takes a note explaining why the thread is blocked.
ALTER TABLE maidan_thread_blocks ADD COLUMN note TEXT;
