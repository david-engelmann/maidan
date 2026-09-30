-- A2A `ListTasks` filters by thread access in the query, so each task records
-- the thread its access follows. Existing rows take the thread the server
-- resolved on read: `metadata.maidan.threadId`, else the thread a client
-- context is bound to, else the context itself when it is a hyphenated
-- UUID. A task none of these name stays on no thread, visible to its
-- workspace as before.
ALTER TABLE maidan_a2a_tasks ADD COLUMN thread_id UUID;

UPDATE maidan_a2a_tasks t SET thread_id = COALESCE(
    CASE WHEN t.task_json->'metadata'->'maidan'->>'threadId'
              ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
         THEN (t.task_json->'metadata'->'maidan'->>'threadId')::uuid END,
    (SELECT c.thread_id FROM maidan_a2a_contexts c
     WHERE c.workspace_id = t.workspace_id AND c.context_id = t.context_id),
    CASE WHEN t.context_id ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
         THEN t.context_id::uuid END
);
