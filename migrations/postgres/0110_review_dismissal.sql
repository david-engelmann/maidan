-- A change request sends a thread back for rework, and the approvals given to
-- the version it replaced stop counting. They are kept, marked dismissed.
ALTER TABLE maidan_thread_reviews ADD COLUMN dismissed_at TIMESTAMPTZ;
