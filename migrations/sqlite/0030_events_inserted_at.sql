-- At-least-once delivery stability horizon (Cluster 125), SQLite dialect.
--
-- See the Postgres 0031 migration for rationale. SQLite forbids a non-constant
-- default (CURRENT_TIMESTAMP) in ALTER TABLE ADD COLUMN, so existing rows take a
-- constant epoch backfill (they are already long-committed and thus stable); the
-- app binds the real insert time on every new append.
ALTER TABLE maidan_events
    ADD COLUMN inserted_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
