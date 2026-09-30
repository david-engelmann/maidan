-- An upload's display name; see the Postgres twin.
ALTER TABLE maidan_artifact_refs ADD COLUMN filename TEXT;
ALTER TABLE maidan_artifacts ADD COLUMN filename TEXT;
