-- Who holds a thread last. `maidan_thread_workers` answers "has this member
-- ever held it?" for separation of duties; a change request also needs "who
-- held it last?", so the worker whose work was sent back hears about it. The
-- live `assignee_id` cannot answer that: a worker releases the claim when it
-- hands the thread to review.
--
-- `last_held_seq` counts per thread: each time a member takes hold, its row
-- gets one more than the thread's highest. A delegate recorded beside the
-- member it acts for gets no number, so the member is the last worker. A count
-- rather than a timestamp, because two claims can share a clock tick.
ALTER TABLE maidan_thread_workers ADD COLUMN IF NOT EXISTS last_held_seq BIGINT;

-- Existing rows keep the order they were first held in, and whoever holds the
-- thread now is last. Earlier re-claims are not recorded anywhere, so this is
-- the best order the data still has.
UPDATE maidan_thread_workers w SET last_held_seq = (
    SELECT COUNT(*) FROM maidan_thread_workers w2
    WHERE w2.thread_id = w.thread_id
      AND (w2.first_held_at < w.first_held_at
           OR (w2.first_held_at = w.first_held_at AND w2.member_id <= w.member_id))
);
UPDATE maidan_thread_workers w SET last_held_seq = (
    SELECT COUNT(*) + 1 FROM maidan_thread_workers w2 WHERE w2.thread_id = w.thread_id
)
FROM maidan_threads t
WHERE t.id = w.thread_id AND t.assignee_id = w.member_id;
