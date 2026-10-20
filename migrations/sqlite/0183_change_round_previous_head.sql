-- A squash-rebase round's pre-move head; see the Postgres twin (0183).
ALTER TABLE maidan_change_rounds ADD COLUMN previous_head TEXT;
