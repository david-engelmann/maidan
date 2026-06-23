-- At-least-once delivery stability horizon (Cluster 125), Postgres dialect.
--
-- `inserted_at` is the DB insert wall-clock (set by the app at append), used by
-- the reconcile read to gate the durable cursor: a row is "stable" only once it
-- is older than the configured window, which guarantees (under "no insert
-- transaction outlives the window") that no lower `id` can still commit and be
-- stranded behind the cursor. Distinct from `occurred_at`, which is the event's
-- caller-supplied logical timestamp and may be skewed/backdated.
ALTER TABLE maidan_events
    ADD COLUMN inserted_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Existing rows are already long-committed (stable); the now() backfill is fine.
