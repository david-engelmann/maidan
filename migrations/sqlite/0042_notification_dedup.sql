-- Cluster 238 (mirror of postgres 0043): dedup guard — one notification per
-- (recipient, source event), so a replayed event or a second replica running the
-- router's bus consumer cannot double-notify (create_notification_if_absent inserts
-- ON CONFLICT DO NOTHING against this unique index).
CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_dedup
    ON maidan_notifications (member_id, source_log_id);
