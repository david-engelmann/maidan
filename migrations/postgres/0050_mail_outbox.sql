-- Durable mail outbox: notification emails are enqueued here and delivered by a
-- retry/backoff worker, instead of a best-effort fire-and-forget send. A crashed
-- or failed send is retried; a permanently-failing message is dead-lettered.
CREATE TABLE IF NOT EXISTS maidan_mail_outbox (
    id UUID PRIMARY KEY,
    to_address TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | dead
    attempts BIGINT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The worker's claim query: oldest due pending row first.
CREATE INDEX IF NOT EXISTS idx_mail_outbox_due
    ON maidan_mail_outbox (next_attempt_at)
    WHERE status = 'pending';
