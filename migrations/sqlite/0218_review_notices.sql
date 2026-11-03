-- A review_submitted notice names who reviewed and how it came out (#1935).
ALTER TABLE maidan_thread_notices ADD COLUMN reviewer TEXT;
ALTER TABLE maidan_thread_notices ADD COLUMN review_state TEXT;
