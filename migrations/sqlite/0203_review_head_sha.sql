-- A verdict names the change's head (S4b); see the Postgres twin.
ALTER TABLE maidan_thread_reviews ADD COLUMN head_sha TEXT;
ALTER TABLE maidan_thread_review_verdicts ADD COLUMN head_sha TEXT;
