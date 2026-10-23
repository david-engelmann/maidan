-- The service lane a thread's model calls may run in (Context Economics C7):
-- 'batch' or 'flex' for work whose deadline has slack. Unset is realtime.
ALTER TABLE maidan_thread_attributes
    ADD COLUMN lane TEXT CHECK (lane IN ('batch', 'flex'));
