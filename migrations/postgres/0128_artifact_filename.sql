-- The name an upload gave its bytes, for display. It is what a workspace said
-- about the bytes, not the bytes, so it lives on that workspace's ref. The
-- shared row carries one only for an upload no workspace owns (bypass, auth
-- off), which has no ref to hold it.
ALTER TABLE maidan_artifact_refs ADD COLUMN filename TEXT;
ALTER TABLE maidan_artifacts ADD COLUMN filename TEXT;
