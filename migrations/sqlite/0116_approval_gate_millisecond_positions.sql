-- A2A `ListTasks` merges pending approval gates into its task list and pages
-- the merged list by `(timestamp, id)`. Task timestamps and page tokens carry
-- milliseconds, so gates now do too: the store can then page gates in exactly
-- the order the listing uses, instead of scanning a fixed number of them.
-- The store writes `created_at` as millisecond `...Z` text; existing rows
-- (RFC 3339 with an offset, or `datetime('now')`) are rewritten to that form
-- so every row compares correctly as a string.
UPDATE maidan_approval_gates
SET created_at = replace(substr(created_at, 1, 19), ' ', 'T') || '.'
    || CASE WHEN substr(created_at, 20, 1) = '.' THEN substr(created_at, 21, 3) ELSE '000' END
    || 'Z';

-- The id breaks ties between gates opened in the same millisecond.
DROP INDEX IF EXISTS idx_approval_gates_pending;
CREATE INDEX idx_approval_gates_pending
    ON maidan_approval_gates (workspace_id, created_at, id);
