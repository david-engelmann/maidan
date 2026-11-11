-- Outbound webhooks, slash commands and A2A push configs act for the
-- credential that registered them, as FSM hooks do since 0232: each fires only
-- while that credential is live and still holds what registering needed, and
-- a webhook delivers only events its creator may read. They kept firing after
-- their creator's token was revoked or its member was deactivated.
--
-- Registrations made before this column have no creator and stop firing on a
-- server with auth enabled; an administrator re-registers the ones still
-- wanted. Deleting the token row leaves the registration without a creator,
-- which stops it the same way.
ALTER TABLE maidan_webhook_subscriptions
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_slash_commands
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
ALTER TABLE maidan_a2a_task_push_configs
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
