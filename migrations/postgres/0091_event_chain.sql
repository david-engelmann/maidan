-- Cluster 392: per-workspace hash chain on the event log.
-- prev_hash / content_hash are sha256:<hex>. Empty until Rust backfill
-- (canonical JSON cannot be hashed in SQL on both dialects).

ALTER TABLE maidan_events
    ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '',
    ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
