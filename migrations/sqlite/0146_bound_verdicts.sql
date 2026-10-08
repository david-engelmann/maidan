-- Open Work Next 3 (evidence-bound approvals), part 3. A verdict records the
-- evidence root of the review packet it was given against. Only an approval
-- bound to a thread's latest packet counts toward its close.
ALTER TABLE maidan_thread_reviews ADD COLUMN evidence_root TEXT;
ALTER TABLE maidan_thread_review_verdicts ADD COLUMN evidence_root TEXT;
