-- Idempotency keys for retried writes (`Idempotency-Key` request header,
-- draft-ietf-httpapi-idempotency-key-header). One row per caller and key:
-- `status` is NULL while the first request runs (a lease until
-- `locked_until`, so a crashed request does not hold the key forever), then
-- the stored response that every retry with the same request gets back.
-- `fingerprint` is a SHA-256 over method, path, query and body; reusing a key
-- for a different request is refused. Rows lapse at `expires_at`.
CREATE TABLE IF NOT EXISTS maidan_idempotency_keys (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    status INTEGER,
    content_type TEXT,
    body BYTEA,
    locked_until TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, actor_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS maidan_idempotency_keys_expires_at
    ON maidan_idempotency_keys (expires_at);
