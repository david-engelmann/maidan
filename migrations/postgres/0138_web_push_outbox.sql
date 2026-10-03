-- Durable web push outbox. A failed send is queued here and retried by the
-- web push worker. Deleting the subscription or the member drops its rows.
CREATE TABLE IF NOT EXISTS maidan_web_push_outbox (
    id UUID PRIMARY KEY,
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    subscription_id UUID NOT NULL REFERENCES maidan_push_subscriptions(id) ON DELETE CASCADE,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts BIGINT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_web_push_outbox_due
    ON maidan_web_push_outbox (next_attempt_at)
    WHERE status = 'pending';
