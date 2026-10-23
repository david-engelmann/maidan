-- The gate last said on a tracked pull request; see the Postgres twin.
ALTER TABLE maidan_pull_ci ADD COLUMN gate_said TEXT;
