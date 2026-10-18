-- The head a squash-rebase round force-moved the agent branch from
-- (crates/maidan-server/src/change_flow.rs). With the round's own commit it
-- is the range a reviewer diffs to see what the rebase changed; the force
-- move rewrote history, so GitHub no longer shows it. NULL for a normal
-- round, which only fast-forwards, and for rounds recorded before this
-- column.
ALTER TABLE maidan_change_rounds ADD COLUMN previous_head TEXT;
