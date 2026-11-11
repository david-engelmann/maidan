-- Webhooks, slash commands and A2A push configs act for the credential that
-- registered them; see the Postgres twin (0234).
ALTER TABLE maidan_webhook_subscriptions
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_slash_commands
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_a2a_task_push_configs
    ADD COLUMN created_by_token TEXT REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
