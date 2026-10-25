-- The service lane a thread may run in (Context Economics C7); see the Postgres twin.
ALTER TABLE maidan_thread_attributes ADD COLUMN lane TEXT CHECK (lane IN ('batch', 'flex'));
