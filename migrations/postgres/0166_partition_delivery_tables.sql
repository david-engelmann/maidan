-- The delivery tables become range-partitioned tables, one partition per
-- calendar month (UTC), the way 0162 partitioned the event log and 0165 the
-- audit log. Postgres only: SQLite keeps one table each.
--
-- The tables are the retention sweep's terminal-rows set (postgres/
-- retention.rs): the ones whose finished rows `prune_deliveries` deletes by
-- age. Each is partitioned on a time column that is set once, so a row
-- never changes partition after it lands:
--
--   maidan_webhook_deliveries     created_at
--   maidan_automation_deliveries  created_at
--   maidan_outbox                 created_at
--   maidan_mail_outbox            updated_at
--   maidan_agent_work_dlq         failed_at
--
-- `maidan_outbox` is the one judgment call. Retention cuts it on
-- `published_at`, but that column is NULL for the whole live queue, and a
-- nullable column cannot key a partition (a NULL key belongs to no month).
-- `created_at` is set once and a row is normally published moments later,
-- so the months line up; where they do not, retention's drop pass checks
-- every row's deletability exactly before dropping (see retention.rs), so
-- a slow-to-publish row keeps its month rather than vanishing with it.
-- Mail's `updated_at` moves while a row retries and stops when the row is
-- terminal, so a month of delivered mail ages in place.
--
-- Two of the sweep's tables are NOT partitioned, deliberately:
-- `maidan_egress_outbox` carries the global unique index
-- `idx_egress_outbox_dedup (source_log_id, surface, selector)` that
-- enqueue idempotency rests on; a unique index on a partitioned table
-- must include the partition key, which would silently weaken dedup to
-- per-month. `maidan_result_deliveries` is updated in place under its own
-- unique key and is never age-pruned; it is state, not a log.
--
-- Each table's primary key must include its partition key, so each becomes
-- `(id, <key>)`; ids still come from one sequence (or are the caller's
-- UUIDs), unique by construction as before. Existing rows are not copied:
-- each old table is renamed and attached as one partition covering
-- everything before the first month after its newest row (and after now),
-- and drains through retention like any partition. Outbound foreign keys
-- are renamed on the legacy table and re-added on the parent, as 0162 did;
-- nothing references any of these tables, so no cascades move.
--
-- The monthly partitions, and each DEFAULT one, are made and kept by
-- `postgres/partitions.rs`, at boot and on every retention sweep (current
-- month plus three ahead), with each table's autovacuum settings.

-- 1. maidan_webhook_deliveries, by created_at.
ALTER TABLE maidan_webhook_deliveries RENAME TO maidan_webhook_deliveries_legacy;
ALTER INDEX maidan_webhook_deliveries_pkey RENAME TO maidan_webhook_deliveries_legacy_id_idx;
ALTER INDEX idx_webhook_deliveries_pending RENAME TO maidan_webhook_deliveries_legacy_pending_idx;
ALTER TABLE maidan_webhook_deliveries_legacy
    RENAME CONSTRAINT maidan_webhook_deliveries_subscription_id_fkey TO maidan_webhook_deliveries_legacy_subscription_id_fkey;

CREATE TABLE maidan_webhook_deliveries (LIKE maidan_webhook_deliveries_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (created_at);
ALTER SEQUENCE maidan_webhook_deliveries_id_seq OWNED BY maidan_webhook_deliveries.id;

ALTER TABLE maidan_webhook_deliveries_legacy DROP CONSTRAINT maidan_webhook_deliveries_legacy_id_idx;
ALTER TABLE maidan_webhook_deliveries_legacy
    ADD CONSTRAINT maidan_webhook_deliveries_legacy_pkey PRIMARY KEY (id, created_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(created_at) INTO newest FROM maidan_webhook_deliveries_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_webhook_deliveries ATTACH PARTITION maidan_webhook_deliveries_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_webhook_deliveries_default PARTITION OF maidan_webhook_deliveries DEFAULT;

ALTER TABLE maidan_webhook_deliveries ADD CONSTRAINT maidan_webhook_deliveries_pkey PRIMARY KEY (id, created_at);
ALTER TABLE maidan_webhook_deliveries
    ADD CONSTRAINT maidan_webhook_deliveries_subscription_id_fkey
    FOREIGN KEY (subscription_id) REFERENCES maidan_webhook_subscriptions(id) ON DELETE CASCADE;
CREATE INDEX idx_webhook_deliveries_pending
    ON maidan_webhook_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;

-- 2. maidan_automation_deliveries, by created_at.
ALTER TABLE maidan_automation_deliveries RENAME TO maidan_automation_deliveries_legacy;
ALTER INDEX maidan_automation_deliveries_pkey RENAME TO maidan_automation_deliveries_legacy_id_idx;
ALTER INDEX idx_automation_deliveries_pending RENAME TO maidan_automation_deliveries_legacy_pending_idx;
ALTER INDEX idx_automation_deliveries_workspace RENAME TO maidan_automation_deliveries_legacy_workspace_idx;
ALTER TABLE maidan_automation_deliveries_legacy
    RENAME CONSTRAINT maidan_automation_deliveries_workspace_id_fkey TO maidan_automation_deliveries_legacy_workspace_id_fkey;

CREATE TABLE maidan_automation_deliveries (LIKE maidan_automation_deliveries_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (created_at);
ALTER SEQUENCE maidan_automation_deliveries_id_seq OWNED BY maidan_automation_deliveries.id;

ALTER TABLE maidan_automation_deliveries_legacy DROP CONSTRAINT maidan_automation_deliveries_legacy_id_idx;
ALTER TABLE maidan_automation_deliveries_legacy
    ADD CONSTRAINT maidan_automation_deliveries_legacy_pkey PRIMARY KEY (id, created_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(created_at) INTO newest FROM maidan_automation_deliveries_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_automation_deliveries ATTACH PARTITION maidan_automation_deliveries_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_automation_deliveries_default PARTITION OF maidan_automation_deliveries DEFAULT;

ALTER TABLE maidan_automation_deliveries ADD CONSTRAINT maidan_automation_deliveries_pkey PRIMARY KEY (id, created_at);
ALTER TABLE maidan_automation_deliveries
    ADD CONSTRAINT maidan_automation_deliveries_workspace_id_fkey
    FOREIGN KEY (workspace_id) REFERENCES maidan_workspaces(id) ON DELETE CASCADE;
CREATE INDEX idx_automation_deliveries_pending
    ON maidan_automation_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;
CREATE INDEX idx_automation_deliveries_workspace
    ON maidan_automation_deliveries (workspace_id, created_at DESC);

-- 3. maidan_outbox, by created_at (see the header for why not published_at).
--    The log_id foreign key went in 0162; there are no constraints to move.
ALTER TABLE maidan_outbox RENAME TO maidan_outbox_legacy;
ALTER INDEX maidan_outbox_pkey RENAME TO maidan_outbox_legacy_id_idx;
ALTER INDEX idx_outbox_relayable RENAME TO maidan_outbox_legacy_relayable_idx;
ALTER INDEX idx_outbox_claimable RENAME TO maidan_outbox_legacy_claimable_idx;
ALTER INDEX idx_outbox_log_id RENAME TO maidan_outbox_legacy_log_id_idx;

CREATE TABLE maidan_outbox (LIKE maidan_outbox_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (created_at);
ALTER SEQUENCE maidan_outbox_id_seq OWNED BY maidan_outbox.id;

ALTER TABLE maidan_outbox_legacy DROP CONSTRAINT maidan_outbox_legacy_id_idx;
ALTER TABLE maidan_outbox_legacy
    ADD CONSTRAINT maidan_outbox_legacy_pkey PRIMARY KEY (id, created_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(created_at) INTO newest FROM maidan_outbox_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_outbox ATTACH PARTITION maidan_outbox_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_outbox_default PARTITION OF maidan_outbox DEFAULT;

ALTER TABLE maidan_outbox ADD CONSTRAINT maidan_outbox_pkey PRIMARY KEY (id, created_at);
CREATE INDEX idx_outbox_relayable ON maidan_outbox (id)
    WHERE published_at IS NULL AND quarantined_at IS NULL;
CREATE INDEX idx_outbox_claimable ON maidan_outbox (id, claimed_at)
    WHERE published_at IS NULL AND quarantined_at IS NULL;
CREATE INDEX idx_outbox_log_id ON maidan_outbox (log_id);

-- 4. maidan_mail_outbox, by updated_at. UUID ids: no sequence to re-own.
ALTER TABLE maidan_mail_outbox RENAME TO maidan_mail_outbox_legacy;
ALTER INDEX maidan_mail_outbox_pkey RENAME TO maidan_mail_outbox_legacy_id_idx;
ALTER INDEX idx_mail_outbox_due RENAME TO maidan_mail_outbox_legacy_due_idx;
ALTER INDEX idx_mail_outbox_workspace_dead RENAME TO maidan_mail_outbox_legacy_workspace_dead_idx;
ALTER INDEX idx_mail_outbox_content_key RENAME TO maidan_mail_outbox_legacy_content_key_idx;
ALTER TABLE maidan_mail_outbox_legacy
    RENAME CONSTRAINT maidan_mail_outbox_workspace_id_fkey TO maidan_mail_outbox_legacy_workspace_id_fkey;
ALTER TABLE maidan_mail_outbox_legacy
    RENAME CONSTRAINT maidan_mail_outbox_content_key_id_fkey TO maidan_mail_outbox_legacy_content_key_id_fkey;

CREATE TABLE maidan_mail_outbox (LIKE maidan_mail_outbox_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (updated_at);

ALTER TABLE maidan_mail_outbox_legacy DROP CONSTRAINT maidan_mail_outbox_legacy_id_idx;
ALTER TABLE maidan_mail_outbox_legacy
    ADD CONSTRAINT maidan_mail_outbox_legacy_pkey PRIMARY KEY (id, updated_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(updated_at) INTO newest FROM maidan_mail_outbox_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_mail_outbox ATTACH PARTITION maidan_mail_outbox_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_mail_outbox_default PARTITION OF maidan_mail_outbox DEFAULT;

ALTER TABLE maidan_mail_outbox ADD CONSTRAINT maidan_mail_outbox_pkey PRIMARY KEY (id, updated_at);
ALTER TABLE maidan_mail_outbox
    ADD CONSTRAINT maidan_mail_outbox_workspace_id_fkey
    FOREIGN KEY (workspace_id) REFERENCES maidan_workspaces(id) ON DELETE CASCADE;
ALTER TABLE maidan_mail_outbox
    ADD CONSTRAINT maidan_mail_outbox_content_key_id_fkey
    FOREIGN KEY (content_key_id) REFERENCES maidan_content_keys(id) ON DELETE CASCADE;
CREATE INDEX idx_mail_outbox_due
    ON maidan_mail_outbox (next_attempt_at)
    WHERE status = 'pending';
CREATE INDEX idx_mail_outbox_workspace_dead
    ON maidan_mail_outbox (workspace_id, updated_at DESC)
    WHERE status = 'dead';
CREATE INDEX idx_mail_outbox_content_key ON maidan_mail_outbox (content_key_id)
    WHERE content_key_id IS NOT NULL;

-- 5. maidan_agent_work_dlq, by failed_at. UUID ids: no sequence to re-own.
ALTER TABLE maidan_agent_work_dlq RENAME TO maidan_agent_work_dlq_legacy;
ALTER INDEX maidan_agent_work_dlq_pkey RENAME TO maidan_agent_work_dlq_legacy_id_idx;
ALTER INDEX idx_agent_work_dlq_channel RENAME TO maidan_agent_work_dlq_legacy_channel_idx;
ALTER TABLE maidan_agent_work_dlq_legacy
    RENAME CONSTRAINT maidan_agent_work_dlq_workspace_id_fkey TO maidan_agent_work_dlq_legacy_workspace_id_fkey;
ALTER TABLE maidan_agent_work_dlq_legacy
    RENAME CONSTRAINT maidan_agent_work_dlq_channel_id_fkey TO maidan_agent_work_dlq_legacy_channel_id_fkey;
ALTER TABLE maidan_agent_work_dlq_legacy
    RENAME CONSTRAINT maidan_agent_work_dlq_thread_id_fkey TO maidan_agent_work_dlq_legacy_thread_id_fkey;
ALTER TABLE maidan_agent_work_dlq_legacy
    RENAME CONSTRAINT maidan_agent_work_dlq_member_id_fkey TO maidan_agent_work_dlq_legacy_member_id_fkey;

CREATE TABLE maidan_agent_work_dlq (LIKE maidan_agent_work_dlq_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (failed_at);

ALTER TABLE maidan_agent_work_dlq_legacy DROP CONSTRAINT maidan_agent_work_dlq_legacy_id_idx;
ALTER TABLE maidan_agent_work_dlq_legacy
    ADD CONSTRAINT maidan_agent_work_dlq_legacy_pkey PRIMARY KEY (id, failed_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(failed_at) INTO newest FROM maidan_agent_work_dlq_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_agent_work_dlq ATTACH PARTITION maidan_agent_work_dlq_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_agent_work_dlq_default PARTITION OF maidan_agent_work_dlq DEFAULT;

ALTER TABLE maidan_agent_work_dlq ADD CONSTRAINT maidan_agent_work_dlq_pkey PRIMARY KEY (id, failed_at);
ALTER TABLE maidan_agent_work_dlq
    ADD CONSTRAINT maidan_agent_work_dlq_workspace_id_fkey
    FOREIGN KEY (workspace_id) REFERENCES maidan_workspaces(id) ON DELETE CASCADE;
ALTER TABLE maidan_agent_work_dlq
    ADD CONSTRAINT maidan_agent_work_dlq_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES maidan_channels(id) ON DELETE CASCADE;
ALTER TABLE maidan_agent_work_dlq
    ADD CONSTRAINT maidan_agent_work_dlq_thread_id_fkey
    FOREIGN KEY (thread_id) REFERENCES maidan_threads(id) ON DELETE CASCADE;
ALTER TABLE maidan_agent_work_dlq
    ADD CONSTRAINT maidan_agent_work_dlq_member_id_fkey
    FOREIGN KEY (member_id) REFERENCES maidan_members(id) ON DELETE CASCADE;
CREATE INDEX idx_agent_work_dlq_channel
    ON maidan_agent_work_dlq (channel_id, failed_at DESC);
