-- The event log becomes a range-partitioned table, one partition per calendar
-- month (UTC) of `occurred_at`. Postgres only: SQLite keeps one table.
--
-- Why `occurred_at`: retention cuts on it (`prune_events`), so a partition
-- whose upper bound is at or before the cutoff holds only rows past the
-- cutoff, and dropping it is the same as deleting them (when the delivery
-- floor and legal holds allow it; see `postgres/retention.rs`).
--
-- The primary key must include the partition key, so it becomes
-- `(id, occurred_at)`. `id` still comes from one sequence, so it is unique
-- by sequence, not by a separate unique constraint, and every read still
-- orders by `id`: cursors, the per-workspace hash chain (`prev_hash`) and
-- the delivery floor are unchanged.
--
-- Existing rows are not copied. The old table is renamed and attached as one
-- partition covering everything before the first month after its newest row
-- (and after now). That costs one read of the table for its newest
-- `occurred_at`, a build of the `(id, occurred_at)` key, and the attach's
-- check of the bound, all under this migration's lock on the table. It drains
-- through retention like any partition: by batched DELETE until every row in
-- it is past the cutoff, then by DROP.
--
-- The monthly partitions, and the DEFAULT one that catches a row no partition
-- covers, are made and kept by `postgres/partitions.rs`, at boot and on every
-- retention sweep (current month plus three ahead), with the table's
-- autovacuum settings.

-- 1. Two tables point at an event by `id` alone, which a key of
--    `(id, occurred_at)` can no longer back. Their ON DELETE CASCADE moves to
--    the trigger below (and to an explicit delete before a partition drop).
ALTER TABLE maidan_outbox DROP CONSTRAINT maidan_outbox_log_id_fkey;
ALTER TABLE maidan_federated_ingest DROP CONSTRAINT maidan_federated_ingest_local_event_id_fkey;
-- The cascade used to find outbox rows by scanning; now it has an index.
CREATE INDEX idx_outbox_log_id ON maidan_outbox (log_id);

-- 2. Keep the old table as the first partition.
ALTER TABLE maidan_events RENAME TO maidan_events_legacy;
ALTER INDEX maidan_events_pkey RENAME TO maidan_events_legacy_id_idx;
ALTER INDEX idx_events_workspace_id RENAME TO maidan_events_legacy_workspace_id_idx;
ALTER INDEX idx_events_content_key RENAME TO maidan_events_legacy_content_key_idx;
ALTER TABLE maidan_events_legacy
    RENAME CONSTRAINT maidan_events_content_key_id_fkey TO maidan_events_legacy_content_key_id_fkey;

CREATE TABLE maidan_events (LIKE maidan_events_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (occurred_at);
-- The id sequence belongs to the parent, so dropping the old partition later
-- does not drop it.
ALTER SEQUENCE maidan_events_id_seq OWNED BY maidan_events.id;

ALTER TABLE maidan_events_legacy DROP CONSTRAINT maidan_events_legacy_id_idx;
ALTER TABLE maidan_events_legacy
    ADD CONSTRAINT maidan_events_legacy_pkey PRIMARY KEY (id, occurred_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(occurred_at) INTO newest FROM maidan_events_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_events ATTACH PARTITION maidan_events_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_events_default PARTITION OF maidan_events DEFAULT;

-- 3. The parent's key, indexes and foreign key. Each attaches the old
--    table's matching index or constraint instead of building another.
ALTER TABLE maidan_events ADD CONSTRAINT maidan_events_pkey PRIMARY KEY (id, occurred_at);
CREATE INDEX idx_events_workspace_id ON maidan_events (workspace_id, id);
CREATE INDEX idx_events_content_key ON maidan_events (content_key_id)
    WHERE content_key_id IS NOT NULL;
ALTER TABLE maidan_events
    ADD CONSTRAINT maidan_events_content_key_id_fkey
    FOREIGN KEY (content_key_id) REFERENCES maidan_content_keys(id);

-- 4. The cascade the two foreign keys did. A partition maintenance move sets
--    `maidan.partition_move` so moving a row out of DEFAULT does not delete
--    what points at it.
CREATE FUNCTION maidan_events_cascade_delete() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('maidan.partition_move', true) = 'on' THEN
        RETURN OLD;
    END IF;
    DELETE FROM maidan_outbox WHERE log_id = OLD.id;
    DELETE FROM maidan_federated_ingest WHERE local_event_id = OLD.id;
    RETURN OLD;
END
$$;

CREATE TRIGGER maidan_events_cascade_delete
    AFTER DELETE ON maidan_events
    FOR EACH ROW EXECUTE FUNCTION maidan_events_cascade_delete();
