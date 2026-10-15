-- The change sweep; see the Postgres twin (0177).
ALTER TABLE maidan_change_pulls ADD COLUMN checked_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN settled_at TEXT;
UPDATE maidan_change_pulls SET settled_at = closed_at WHERE closed_at IS NOT NULL;
