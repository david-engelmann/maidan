-- A workspace's own media retention and media quota. Media artifacts
-- (attachments, screenshots, recordings) fill storage fast, so an unreferenced
-- one expires after media_days, and an upload that would take the workspace's
-- media past media_quota_bytes is refused. NULL keeps the instance's
-- MAIDAN_RETENTION_MEDIA_DAYS and MAIDAN_MEDIA_QUOTA_BYTES. Bounded by those at
-- write, like the other retention columns. Evidence kinds are never touched.
ALTER TABLE maidan_retention_policies
    ADD COLUMN media_days BIGINT CHECK (media_days BETWEEN 1 AND 3650),
    ADD COLUMN media_quota_bytes BIGINT CHECK (media_quota_bytes > 0);

-- When this workspace last uploaded these bytes. A media ref's TTL runs from
-- here, not from created_at, which keeps the first upload: the same
-- screenshot uploaded again today must not expire before the message about
-- it is posted. NULL on rows from before this migration reads as created_at.
ALTER TABLE maidan_artifact_refs ADD COLUMN last_uploaded_at TIMESTAMPTZ;
