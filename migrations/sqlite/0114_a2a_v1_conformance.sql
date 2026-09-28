-- A2A v1.0 conformance. See the Postgres 0115 migration for the rationale.
ALTER TABLE maidan_a2a_tasks ADD COLUMN context_id TEXT;
ALTER TABLE maidan_a2a_tasks ADD COLUMN state TEXT NOT NULL DEFAULT '';
UPDATE maidan_a2a_tasks SET
    context_id = json_extract(task_json, '$.contextId'),
    state = COALESCE(json_extract(task_json, '$.status.state'), ''),
    task_json = json_set(
        json_remove(task_json, '$.status.message'),
        '$.status.timestamp',
        updated_at
    );
DROP INDEX idx_maidan_a2a_tasks_workspace;
CREATE INDEX idx_maidan_a2a_tasks_page
    ON maidan_a2a_tasks (workspace_id, updated_at DESC, id DESC);
CREATE INDEX idx_maidan_a2a_tasks_context
    ON maidan_a2a_tasks (workspace_id, context_id);

CREATE TABLE maidan_a2a_contexts (
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    context_id   TEXT NOT NULL,
    thread_id    TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (workspace_id, context_id)
);

ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN token_ciphertext TEXT;
ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN auth_scheme TEXT;
ALTER TABLE maidan_a2a_task_push_configs ADD COLUMN auth_credentials_ciphertext TEXT;
