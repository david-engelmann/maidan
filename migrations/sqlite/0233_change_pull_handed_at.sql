-- A superseded task hands its pull request to its successor; see the Postgres
-- twin (0233).
ALTER TABLE maidan_change_pulls ADD COLUMN handed_at TEXT;
