-- Durable web push outbox. A failed send is queued here and retried by the
-- web push worker. Deleting the subscription or the member drops its rows.
CREATE TABLE IF NOT EXISTS maidan_web_push_outbox (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    subscription_id TEXT NOT NULL REFERENCES maidan_push_subscriptions(id) ON DELETE CASCADE,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_push_outbox_due
    ON maidan_web_push_outbox (next_attempt_at)
    WHERE status = 'pending';
