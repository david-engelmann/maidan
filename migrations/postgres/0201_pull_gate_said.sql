-- The gate last said on a tracked pull request (gate_blocked's reasons, or
-- [] after gate_clear), apart from the last read's `gate_reasons`. A gate is
-- said once two reads agree and it differs from this, so GitHub's merge
-- state flickering for one read says nothing. NULL until a gate is said; an
-- existing row says its next held gate once.
ALTER TABLE maidan_pull_ci ADD COLUMN gate_said JSONB;
