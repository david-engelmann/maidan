-- Maidan nested threads, v0005, Postgres dialect.

ALTER TABLE maidan_threads
    ADD COLUMN parent_thread_id UUID REFERENCES maidan_threads(id) ON DELETE SET NULL;

CREATE INDEX idx_threads_parent ON maidan_threads (parent_thread_id);
