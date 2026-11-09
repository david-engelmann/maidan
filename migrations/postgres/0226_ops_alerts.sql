-- Operator alerts that stand for a condition rather than an event (GitHub
-- refusing a repository's checks; the build that last announced itself on
-- boot). The hosted container sleeps after 15 idle minutes and every wake is
-- a fresh process, so an in-memory dedupe reposted the same standing alert
-- after each wake. One row per alert key while it stands: raising a key that
-- already holds the same value posts nothing, and clearing it deletes the
-- row, so the condition coming back posts again. Not tenant data: the keys
-- are the operator's, like the Slack channel they post to.
CREATE TABLE IF NOT EXISTS maidan_ops_alerts (
    alert_key TEXT PRIMARY KEY,
    value     TEXT NOT NULL,
    raised_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
