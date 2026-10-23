-- A verdict names the full head SHA of the change it reviewed (S4b): the
-- tracked pull's head when the verdict was given. A push that moves the head
-- dismisses the approvals given to an earlier one.
ALTER TABLE maidan_thread_reviews ADD COLUMN head_sha TEXT;
ALTER TABLE maidan_thread_review_verdicts ADD COLUMN head_sha TEXT;
