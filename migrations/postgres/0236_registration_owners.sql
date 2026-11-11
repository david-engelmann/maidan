-- A webhook subscription, a slash command and an A2A push config act for the
-- credential that registered it, and stop when that credential does, as an
-- FSM hook has since 0232. None of them recorded who made it, so revoking or
-- deactivating that member left the registration firing, and an administrator
-- could not tell whose it was.
--
-- Registrations made before this column have no creator and are deleted: with
-- auth enabled they could never fire again. An administrator re-registers the
-- ones still wanted. Deleting a token row leaves its registrations without a
-- creator, which stops them the same way.
DELETE FROM maidan_webhook_subscriptions;
DELETE FROM maidan_slash_commands;
DELETE FROM maidan_a2a_task_push_configs;

ALTER TABLE maidan_webhook_subscriptions
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_slash_commands
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_a2a_task_push_configs
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
