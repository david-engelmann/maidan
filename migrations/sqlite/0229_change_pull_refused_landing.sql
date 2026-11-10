-- A merge whose landing a gate refused is tried again; see the Postgres twin (0229).
ALTER TABLE maidan_change_pulls ADD COLUMN refused_at TEXT;
ALTER TABLE maidan_change_pulls ADD COLUMN retried_at TEXT;
