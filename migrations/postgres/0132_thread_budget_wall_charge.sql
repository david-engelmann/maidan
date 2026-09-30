-- Seconds worked by claims on a thread whose lease lapsed, from each one's
-- acknowledgement to its deadline. The claim reaper (or the claim that takes
-- over a lapsed lease) charges it in the transaction that frees the claim, so
-- max_wall_secs binds on a hung agent that never reports. The live claim's
-- share still derives from maidan_threads.work_started_at.
ALTER TABLE maidan_thread_budgets
    ADD COLUMN IF NOT EXISTS used_wall_secs BIGINT NOT NULL DEFAULT 0;
