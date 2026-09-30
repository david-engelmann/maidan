-- Who holds a thread last. See the Postgres twin.
ALTER TABLE maidan_thread_workers ADD COLUMN last_held_seq INTEGER;

UPDATE maidan_thread_workers SET last_held_seq = (
    SELECT COUNT(*) FROM maidan_thread_workers w2
    WHERE w2.thread_id = maidan_thread_workers.thread_id
      AND (w2.first_held_at < maidan_thread_workers.first_held_at
           OR (w2.first_held_at = maidan_thread_workers.first_held_at
               AND w2.member_id <= maidan_thread_workers.member_id))
);
UPDATE maidan_thread_workers SET last_held_seq = (
    SELECT COUNT(*) + 1 FROM maidan_thread_workers w2
    WHERE w2.thread_id = maidan_thread_workers.thread_id
)
WHERE EXISTS (
    SELECT 1 FROM maidan_threads t
    WHERE t.id = maidan_thread_workers.thread_id
      AND t.assignee_id = maidan_thread_workers.member_id
);
