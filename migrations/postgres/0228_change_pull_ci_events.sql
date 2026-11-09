-- CI read from GitHub's events, not polled (crates/maidan-server/src/change_sweep.rs).
--
-- A fine-grained token cannot read the GraphQL `statusCheckRollup`, so the
-- sweep fell back to the Actions and commit-status APIs for every open
-- tracked pull request on every pass, and that spent the instance's shared
-- REST budget. With the GitHub ingress configured, the sweep reads a pull
-- request's CI only when one of these says it may have changed:
--
-- `ci_event_at`: the last `workflow_run`, `check_suite`, `check_run` or
--   `status` event GitHub delivered for the pull request or its head.
-- `ci_read_at`: when the sweep's last read of its CI began. A read that
--   began before an event arrived is followed by another.
-- `ci_read_head`: the head that read was at, so a push is read at once and
--   an event naming only the head finds the pull request.
ALTER TABLE maidan_change_pulls
    ADD COLUMN ci_event_at TIMESTAMPTZ,
    ADD COLUMN ci_read_at TIMESTAMPTZ,
    ADD COLUMN ci_read_head TEXT;
-- An event names a head, not a workspace: the open pull requests at it.
CREATE INDEX IF NOT EXISTS maidan_change_pulls_ci_read_head
    ON maidan_change_pulls (repo, ci_read_head) WHERE closed_at IS NULL;
