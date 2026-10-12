-- A workspace's own media retention and media quota. Media artifacts
-- (attachments, screenshots, recordings) fill storage fast, so an unreferenced
-- one expires after media_days, and an upload that would take the workspace's
-- media past media_quota_bytes is refused. NULL keeps the instance's
-- MAIDAN_RETENTION_MEDIA_DAYS and MAIDAN_MEDIA_QUOTA_BYTES. Bounded by those at
-- write, like the other retention columns. Evidence kinds are never touched.
ALTER TABLE maidan_retention_policies
    ADD COLUMN media_days BIGINT CHECK (media_days BETWEEN 1 AND 3650),
    ADD COLUMN media_quota_bytes BIGINT CHECK (media_quota_bytes > 0);
