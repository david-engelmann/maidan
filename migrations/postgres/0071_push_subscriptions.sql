-- Cluster 366 (Wave 1 #14, N1): Web Push subscriptions. A browser's
-- PushManager.subscribe() yields {endpoint, keys:{p256dh, auth}}; we store one row
-- per member device. The notification router delivers a Web Push message to these
-- endpoints when the member has no live WebSocket connection. Keys are the
-- subscription's public ECDH point (p256dh) and auth secret, base64url-encoded.
CREATE TABLE IF NOT EXISTS maidan_push_subscriptions (
    id          UUID PRIMARY KEY,
    member_id   UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    endpoint    TEXT NOT NULL,
    p256dh      TEXT NOT NULL,
    auth        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (member_id, endpoint)
);
CREATE INDEX IF NOT EXISTS idx_push_subscriptions_member
    ON maidan_push_subscriptions (member_id);
