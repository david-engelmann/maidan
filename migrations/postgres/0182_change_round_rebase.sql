-- Squash-rebase rounds (crates/maidan-server/src/change_flow.rs).
--
-- `rebase` marks a round that rebuilt the agent branch as one commit on the
-- base's head, so the swarm metrics can count rebases per task and tell a
-- rebase's whole-branch diff apart from a growing one. Nullable: rounds
-- recorded before this column were never rebases, and NULL reads as false.
--
-- `workspace_id` is the workspace that delivered the round. A squash-rebase
-- may force-move a branch only while its head is the newest round *this*
-- workspace delivered; the pull request row cannot say that, because a round
-- from another workspace on the same pull request is filed under it.
-- Backfilled from the pull request for rounds recorded before.
ALTER TABLE maidan_change_rounds
    ADD COLUMN rebase BOOLEAN,
    ADD COLUMN workspace_id UUID REFERENCES maidan_workspaces(id) ON DELETE CASCADE;

UPDATE maidan_change_rounds r
SET workspace_id = p.workspace_id
FROM maidan_change_pulls p
WHERE p.repo = r.repo AND p.pull_number = r.pull_number;
