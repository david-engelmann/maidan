-- Durable mail outbox (see postgres/0050). SQLite mirror: timestamps are rfc3339
-- text bound by the store.
CREATE TABLE IF NOT EXISTS maidan_mail_outbox (
    id TEXT PRIMARY KEY,
    to_address TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | dead
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mail_outbox_due
    ON maidan_mail_outbox (next_attempt_at)
    WHERE status = 'pending';
