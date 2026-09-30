-- A2A `ListTasks` filters by thread access in the query, so each task records
-- the thread its access follows. Existing rows take the thread the server
-- resolved on read: `metadata.maidan.threadId`, else the thread a client
-- context is bound to, else the context itself when it is a hyphenated
-- UUID. A task none of these name stays on no thread, visible to its
-- workspace as before.
-- Ids are stored as 16-byte blobs, so a UUID's text form is unhexed.
ALTER TABLE maidan_a2a_tasks ADD COLUMN thread_id TEXT;

UPDATE maidan_a2a_tasks SET thread_id = COALESCE(
    CASE WHEN length(json_extract(task_json, '$.metadata.maidan.threadId')) = 36
          AND length(replace(json_extract(task_json, '$.metadata.maidan.threadId'), '-', '')) = 32
         THEN unhex(replace(json_extract(task_json, '$.metadata.maidan.threadId'), '-', '')) END,
    (SELECT c.thread_id FROM maidan_a2a_contexts c
     WHERE c.workspace_id = maidan_a2a_tasks.workspace_id
       AND c.context_id = maidan_a2a_tasks.context_id),
    CASE WHEN length(context_id) = 36 AND length(replace(context_id, '-', '')) = 32
         THEN unhex(replace(context_id, '-', '')) END
);
