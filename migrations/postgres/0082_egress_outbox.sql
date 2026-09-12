-- Durable projector egress (Cluster 377.1): a Maidan message bound for an external
-- surface (a Slack channel, a GitHub issue/PR) is enqueued here and delivered by a
-- retry/backoff worker, instead of the best-effort inline post the Slack (309) and
-- GitHub (312) projectors did — where a transient 502 dropped the message with
-- nothing but a log line. Modelled on `maidan_mail_outbox` (0050).
--
-- `surface` + `selector` are the destination: `slack` + a channel id, or `github` +
-- `owner/name#123`. They are written from a typed `EgressTarget` and decoded back to
-- one by the worker, so the pair always round-trips.
--
-- `source_log_id` is the `maidan_events` row that caused the send. It carries no
-- foreign key — deliberately, like `maidan_notifications.source_log_id` (0042) — so
-- retention pruning of the event log cannot cascade into a queued delivery.
CREATE TABLE IF NOT EXISTS maidan_egress_outbox (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    source_log_id BIGINT NOT NULL,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | dead
    attempts BIGINT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The worker's claim query: oldest due pending row first.
CREATE INDEX IF NOT EXISTS idx_egress_outbox_due
    ON maidan_egress_outbox (next_attempt_at)
    WHERE status = 'pending';

-- Dedup guard, the Cluster-238 lesson: every server replica runs the notification
-- router's bus consumer, so the same event reaches each one. Enqueueing
-- `ON CONFLICT DO NOTHING` against this index means a 3-replica deploy posts one
-- comment, not three — and a replayed event does not re-post.
CREATE UNIQUE INDEX IF NOT EXISTS idx_egress_outbox_dedup
    ON maidan_egress_outbox (source_log_id, surface, selector);
