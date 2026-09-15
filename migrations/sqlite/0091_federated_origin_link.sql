-- Cluster 392.3: origin chain fields for sequential federation ingest verify.

ALTER TABLE maidan_federated_ingest ADD COLUMN origin_prev_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE maidan_federated_ingest ADD COLUMN origin_content_hash TEXT NOT NULL DEFAULT '';
