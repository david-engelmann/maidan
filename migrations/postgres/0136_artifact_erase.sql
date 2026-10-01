-- Artifacts are erased, never soft-deleted. `tombstoned_at` on
-- maidan_artifacts was never written outside tests. Drop it. A row stays;
-- removing an artifact deletes that workspace's ref, and the row goes with
-- the last ref.
ALTER TABLE maidan_artifacts DROP COLUMN tombstoned_at;
