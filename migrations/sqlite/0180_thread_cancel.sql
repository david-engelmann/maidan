-- The member who created a thread; see the Postgres twin (0180).
ALTER TABLE maidan_threads
    ADD COLUMN created_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;

CREATE TABLE maidan_thread_cancellations (
    thread_id    TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    cancelled_by TEXT NOT NULL REFERENCES maidan_members(id),
    cancelled_at TEXT NOT NULL
);
