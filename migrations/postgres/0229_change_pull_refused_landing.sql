-- A merge whose landing a gate refused is tried again
-- (crates/maidan-server/src/change_sweep.rs).
--
-- A merged change lands through its thread's gates: separation of duties, the
-- review requirement, the land gate. A gate that refused was recorded by
-- settling the close, so the merge was never looked at again, and when the
-- gate was met later (a review approved, the land gate passed) the thread
-- stayed open and every thread depending on it kept waiting. A refused landing
-- now stays unsettled and each sweep tries it again.
--
-- `refused_at`: when the landing was first refused. Set once, so the refusal
--   is said once however many sweeps try again.
-- `retried_at`: when the landing was last tried and refused. The settle queue
--   takes closes never tried first, then the refusals tried longest ago, so a
--   backlog of refusals starves neither new closes nor one another.
ALTER TABLE maidan_change_pulls
    ADD COLUMN refused_at TIMESTAMPTZ,
    ADD COLUMN retried_at TIMESTAMPTZ;
