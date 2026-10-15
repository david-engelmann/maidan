-- The member who created a thread. A producer may cancel a task it created
-- without holding `thread:transition`, so the creator has to be durable and
-- separate from `owner_id`, which can be reassigned. Nullable: threads made
-- before this column, and threads the system makes itself, have no creator.
-- ON DELETE SET NULL so removing a member never blocks on the threads it made.
ALTER TABLE maidan_threads
    ADD COLUMN created_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL;
