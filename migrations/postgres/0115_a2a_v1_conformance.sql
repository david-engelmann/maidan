-- A2A v1.0 conformance.
--
-- Tasks keep no message words: history is rendered from the sealed message
-- log on read, so a shredded message leaves no copy in task_json. Existing
-- rows drop their echoed status message and gain a status timestamp. The
-- context and state columns let ListTasks filter and page (keyset on
-- updated_at, id) in SQL.
ALTER TABLE maidan_a2a_tasks ADD COLUMN context_id TEXT;
ALTER TABLE maidan_a2a_tasks ADD COLUMN state TEXT NOT NULL DEFAULT '';
UPDATE maidan_a2a_tasks SET
    context_id = task_json->>'contextId',
    state = COALESCE(task_json->'status'->>'state', ''),
    task_json = jsonb_set(
        task_json #- '{status,message}',
        '{status,timestamp}',
        to_jsonb(to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'))
    );
DROP INDEX idx_maidan_a2a_tasks_workspace;
CREATE INDEX idx_maidan_a2a_tasks_page
    ON maidan_a2a_tasks (workspace_id, updated_at DESC, id DESC);
CREATE INDEX idx_maidan_a2a_tasks_context
    ON maidan_a2a_tasks (workspace_id, context_id);

-- Client-chosen contextIds, each bound to the thread that carries it.
CREATE TABLE maidan_a2a_contexts (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    context_id   TEXT NOT NULL,
    thread_id    UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, context_id)
);

-- The spec's per-config token and authentication, secrets sealed at rest.
ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN token_ciphertext TEXT;
ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN auth_scheme TEXT;
ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN auth_credentials_ciphertext TEXT;
