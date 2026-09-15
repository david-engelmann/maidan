-- Cluster 392.3: origin chain fields for sequential federation ingest verify.
-- Local append after remap mints a new log id / hashes; these columns keep
-- the *origin* EventLink so the next envelope can be checked without trusting
-- the sender.

ALTER TABLE maidan_federated_ingest
    ADD COLUMN origin_prev_hash TEXT NOT NULL DEFAULT '',
    ADD COLUMN origin_content_hash TEXT NOT NULL DEFAULT '';
