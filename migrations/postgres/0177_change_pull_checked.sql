-- The change sweep (crates/maidan-server/src/change_sweep.rs). `checked_at`
-- orders the refresh, so the pull requests read longest ago go first and a
-- workspace with more than a tick's worth is still read in full over a few
-- ticks. `settled_at` makes a close act on its thread once: a merge lands it,
-- a close without one posts a single notice.
ALTER TABLE maidan_change_pulls
    ADD COLUMN checked_at TIMESTAMPTZ,
    ADD COLUMN settled_at TIMESTAMPTZ;
-- A close recorded before the sweep existed already had whatever attention
-- it got; settling it now would land a thread someone may have reopened.
UPDATE maidan_change_pulls SET settled_at = closed_at WHERE closed_at IS NOT NULL;
