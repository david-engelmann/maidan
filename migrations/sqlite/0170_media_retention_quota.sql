-- A workspace's own media retention and media quota. See the Postgres twin.
ALTER TABLE maidan_retention_policies
    ADD COLUMN media_days INTEGER CHECK (media_days BETWEEN 1 AND 3650);
ALTER TABLE maidan_retention_policies
    ADD COLUMN media_quota_bytes INTEGER CHECK (media_quota_bytes > 0);
ALTER TABLE maidan_artifact_refs ADD COLUMN last_uploaded_at TEXT;
