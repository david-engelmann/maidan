-- Squash-rebase rounds; see the Postgres twin (0182).
ALTER TABLE maidan_change_rounds ADD COLUMN rebase INTEGER;
ALTER TABLE maidan_change_rounds
    ADD COLUMN workspace_id TEXT REFERENCES maidan_workspaces(id) ON DELETE CASCADE;

UPDATE maidan_change_rounds
SET workspace_id = (
    SELECT p.workspace_id FROM maidan_change_pulls p
    WHERE p.repo = maidan_change_rounds.repo AND p.pull_number = maidan_change_rounds.pull_number
);
