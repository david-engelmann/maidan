-- Dismissed approvals; see the Postgres twin (0110).
ALTER TABLE maidan_thread_reviews ADD COLUMN dismissed_at TEXT;
