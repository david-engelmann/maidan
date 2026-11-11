-- One delivery per event, however many replicas see it (CONC-003).
--
-- Every replica runs the webhook worker and the FSM hook worker, and the
-- Postgres bus fans each event out to every replica's LISTEN. So each replica
-- enqueued its own webhook delivery and its own FSM hook delivery for the same
-- event, and ran each MCP-tool hook once per replica; a lag resume from the log
-- enqueued them again. 0095 leased the outbox relay, which stopped one replica
-- publishing an event twice, but not N replicas each acting on one publish.
--
-- The key is the event the delivery is for. Both delivery tables are
-- partitioned by `created_at` (0166), and a unique index on a partitioned table
-- must include the partition key, so a delivery made for an event takes the
-- event's own `occurred_at` as its `created_at`: every replica computes the
-- same value for the same log id, the row lands in the same partition, and
-- `(subscription_id, log_id, created_at)` is as good as `(subscription_id,
-- log_id)`. A slash-command delivery has no event (`log_id` NULL) and is
-- unaffected, NULLs being distinct.
--
-- Existing duplicates are removed first, keeping the oldest row per event.

-- 1. Webhook deliveries: one per (subscription, event).
DELETE FROM maidan_webhook_deliveries d
USING maidan_webhook_deliveries keep
WHERE keep.subscription_id = d.subscription_id
  AND keep.log_id = d.log_id
  AND keep.id < d.id;

CREATE UNIQUE INDEX idx_webhook_deliveries_event
    ON maidan_webhook_deliveries (subscription_id, log_id, created_at);

-- 2. Automation deliveries: an FSM hook's HTTP delivery names its event. No
--    existing row has one, so there is nothing to deduplicate.
ALTER TABLE maidan_automation_deliveries ADD COLUMN log_id BIGINT;

CREATE UNIQUE INDEX idx_automation_deliveries_event
    ON maidan_automation_deliveries (source_kind, source_id, log_id, created_at);

-- 3. FSM hook runs: the claim a replica takes before it runs a hook for an
--    event, whatever the handler. An MCP-tool hook runs in the worker, not
--    through a delivery row, so without this every replica ran the tool.
CREATE TABLE maidan_fsm_hook_runs (
    id            BIGSERIAL PRIMARY KEY,
    workspace_id  UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    hook_id       UUID NOT NULL REFERENCES maidan_fsm_hooks(id) ON DELETE CASCADE,
    log_id        BIGINT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (hook_id, log_id)
);

CREATE INDEX idx_fsm_hook_runs_workspace
    ON maidan_fsm_hook_runs (workspace_id, created_at);
