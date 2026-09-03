-- Cluster 351 (mirror of postgres 0058): the working clock (rfc3339 TEXT in
-- SQLite). NULL until `acknowledge_claim`; reset on every (re)claim/unassign.
ALTER TABLE maidan_threads ADD COLUMN work_started_at TEXT;
