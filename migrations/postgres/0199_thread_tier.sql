-- A change's review tier (WAX-D-079), a task attribute set at intake and
-- restated when a new head changes it. Unset is kept apart from 'B': the
-- metrics count an unset tier as B, the stored value does not.
ALTER TABLE maidan_thread_attributes
    ADD COLUMN tier TEXT CHECK (tier IN ('A', 'B'));
