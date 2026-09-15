-- Cluster 392: per-workspace hash chain on the event log.
-- prev_hash / content_hash are sha256:<hex>. Empty until Rust backfill.

ALTER TABLE maidan_events ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE maidan_events ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
