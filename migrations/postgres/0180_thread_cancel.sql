-- The member who created a thread. A producer may cancel a task it created
-- without holding `thread:transition`, so the creator has to be durable and
-- separate from `owner_id`, which can be reassigned. Nullable: threads made
-- before this column, and threads the system makes itself, have no creator.
-- ON DELETE SET NULL so removing a member never blocks on the threads it made.
ALTER TABLE maidan_threads
    ADD COLUMN created_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL;

-- One row per cancelled thread. A cancel closes the thread, and a closed
-- thread does not say why; this row is how a repeated cancel is told apart
-- from a thread that closed some other way.
CREATE TABLE maidan_thread_cancellations (
    thread_id    UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    cancelled_by UUID NOT NULL REFERENCES maidan_members(id),
    cancelled_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
