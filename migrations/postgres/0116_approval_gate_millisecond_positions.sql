-- A2A `ListTasks` merges pending approval gates into its task list and pages
-- the merged list by `(timestamp, id)`. Task timestamps and page tokens carry
-- milliseconds, so gates now do too: the store can then page gates in exactly
-- the order the listing uses, instead of scanning a fixed number of them.
UPDATE maidan_approval_gates SET created_at = date_trunc('milliseconds', created_at);
ALTER TABLE maidan_approval_gates
    ALTER COLUMN created_at SET DEFAULT date_trunc('milliseconds', now());

-- The id breaks ties between gates opened in the same millisecond.
DROP INDEX IF EXISTS idx_approval_gates_pending;
CREATE INDEX idx_approval_gates_pending
    ON maidan_approval_gates (workspace_id, created_at, id)
    WHERE state = 'pending';
