-- A registration acts for the credential that registered it; see the
-- Postgres twin (0236).
DELETE FROM maidan_webhook_subscriptions;
DELETE FROM maidan_slash_commands;
DELETE FROM maidan_a2a_task_push_configs;

ALTER TABLE maidan_webhook_subscriptions
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_slash_commands
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_a2a_task_push_configs
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
