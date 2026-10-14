-- The audit log becomes a range-partitioned table, one partition per calendar
-- month (UTC) of `occurred_at`, the way 0162 partitioned the event log.
-- Postgres only: SQLite keeps one table.
--
-- Why `occurred_at`: audit retention cuts on it (`prune_audit`), so a
-- partition whose upper bound is at or before the cutoff holds only rows
-- past the cutoff, and dropping it is the same as deleting them (when no
-- legal hold covers a workspace in it; see `postgres/retention.rs`).
--
-- The primary key must include the partition key, so it becomes
-- `(id, occurred_at)`. `id` still comes from one sequence, so it is unique
-- by sequence, not by a separate unique constraint, and every read still
-- orders by `id` where it did.
--
-- Nothing points at an audit row and, since 0108, the audit table itself
-- holds no foreign keys, so there are no cascades to move to a trigger
-- (0162 had two). Existing rows are not copied: the old table is renamed
-- and attached as one partition covering everything before the first month
-- after its newest row (and after now), and drains through retention like
-- any partition.
--
-- The monthly partitions, and the DEFAULT one that catches a row no
-- partition covers, are made and kept by `postgres/partitions.rs`, at boot
-- and on every retention sweep (current month plus three ahead), with the
-- table's autovacuum settings.

-- 1. Keep the old table as the first partition.
ALTER TABLE maidan_audit RENAME TO maidan_audit_legacy;
ALTER INDEX maidan_audit_pkey RENAME TO maidan_audit_legacy_id_idx;
ALTER INDEX idx_audit_occurred RENAME TO maidan_audit_legacy_occurred_idx;
ALTER INDEX idx_audit_actor RENAME TO maidan_audit_legacy_actor_idx;
ALTER INDEX idx_audit_subject RENAME TO maidan_audit_legacy_subject_idx;
ALTER INDEX idx_audit_grant RENAME TO maidan_audit_legacy_grant_idx;
ALTER INDEX idx_audit_workspace RENAME TO maidan_audit_legacy_workspace_idx;

CREATE TABLE maidan_audit (LIKE maidan_audit_legacy INCLUDING DEFAULTS)
    PARTITION BY RANGE (occurred_at);
-- The id sequence belongs to the parent, so dropping the old partition later
-- does not drop it.
ALTER SEQUENCE maidan_audit_id_seq OWNED BY maidan_audit.id;

ALTER TABLE maidan_audit_legacy DROP CONSTRAINT maidan_audit_legacy_id_idx;
ALTER TABLE maidan_audit_legacy
    ADD CONSTRAINT maidan_audit_legacy_pkey PRIMARY KEY (id, occurred_at);

DO $$
DECLARE
    newest timestamptz;
    bound  timestamptz;
BEGIN
    SELECT max(occurred_at) INTO newest FROM maidan_audit_legacy;
    bound := date_trunc('month', GREATEST(now(), COALESCE(newest, now())), 'UTC')
             + interval '1 month';
    EXECUTE format(
        'ALTER TABLE maidan_audit ATTACH PARTITION maidan_audit_legacy FOR VALUES FROM (MINVALUE) TO (%L)',
        bound);
END
$$;

CREATE TABLE maidan_audit_default PARTITION OF maidan_audit DEFAULT;

-- 2. The parent's key and indexes. Each is created on the parent and so on
--    every partition, the legacy one included.
ALTER TABLE maidan_audit ADD CONSTRAINT maidan_audit_pkey PRIMARY KEY (id, occurred_at);
CREATE INDEX idx_audit_occurred ON maidan_audit (occurred_at DESC);
CREATE INDEX idx_audit_actor ON maidan_audit (actor_id);
CREATE INDEX idx_audit_subject ON maidan_audit (subject_id);
CREATE INDEX idx_audit_grant ON maidan_audit (grant_id) WHERE grant_id IS NOT NULL;
CREATE INDEX idx_audit_workspace ON maidan_audit (workspace_id, occurred_at DESC);
